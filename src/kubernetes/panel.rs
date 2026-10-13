use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    commands,
    details::KubernetesDetails,
    model::{self, KubernetesCommand, Resource, ResourceKind},
    presentation::{copy_button, reading, tone_color},
};
use crate::app::KubernetesAction;
use crate::i18n::{UiLocale, t};
use crate::shared::soft_tag;
use crate::terminal::{ExecTarget, exec_answer};

/// Retains a single terminal's scoped browser. A change of SSH target
/// clears its snapshots; every async result is guarded by a generation.
pub struct KubernetesPanel {
    target: Option<ExecTarget>,
    active: bool,
    context: Option<String>,
    contexts: Vec<String>,
    namespace: Option<String>,
    namespaces: Vec<String>,
    namespaces_ready: bool,
    kind: ResourceKind,
    rows: Rc<Vec<Resource>>,
    selected: Option<String>,
    context_select: Entity<SelectState<Vec<SharedString>>>,
    namespace_select: Entity<SelectState<Vec<SharedString>>>,
    kind_select: Entity<SelectState<Vec<SharedString>>>,
    search: Entity<InputState>,
    query: String,
    loading: bool,
    problem: Option<String>,
    generation: u64,
    task: Option<Task<()>>,
    details: Option<WeakEntity<KubernetesDetails>>,
    scroll: UniformListScrollHandle,
    dispatch: FocusHandle,
    focus: FocusHandle,
    window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl KubernetesPanel {
    pub fn new(dispatch: FocusHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let context_select = cx.new(|cx| {
            SelectState::new(Vec::<SharedString>::new(), None, window, cx).searchable(true)
        });
        let namespace_select = cx.new(|cx| {
            SelectState::new(vec![t!("k8s.all")], Some(IndexPath::new(0)), window, cx)
                .searchable(true)
        });
        let kind_select = cx.new(|cx| {
            SelectState::new(
                ResourceKind::ALL
                    .iter()
                    .map(|kind| SharedString::from(kind.label()))
                    .collect::<Vec<_>>(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("k8s.search"))
                .clean_on_escape()
        });
        let mut subscriptions = Vec::new();
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &context_select,
            window,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Context(value.to_string())),
                        window,
                        cx,
                    );
                }
            },
        ));
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &namespace_select,
            window,
            move |_, state, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let namespace = (state.read(cx).selected_index(cx) != Some(IndexPath::new(0)))
                        .then(|| value.to_string());
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Namespace(namespace)),
                        window,
                        cx,
                    );
                }
            },
        ));
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &kind_select,
            window,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && let Some(kind) = ResourceKind::ALL
                        .iter()
                        .find(|kind| kind.label() == value.as_ref())
                {
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Kind(*kind)),
                        window,
                        cx,
                    );
                }
            },
        ));
        subscriptions.push(
            cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = state.read(cx).value().to_string();
                    this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.observe_global_in::<UiLocale>(window, |this, window, cx| {
                this.search.update(cx, |input, cx| {
                    input.set_placeholder(t!("k8s.search"), window, cx)
                });
                this.sync_selects(window, cx);
            }),
        );
        Self {
            target: None,
            active: false,
            context: None,
            contexts: Vec::new(),
            namespace: None,
            namespaces: Vec::new(),
            namespaces_ready: false,
            kind: ResourceKind::Pod,
            rows: Rc::new(Vec::new()),
            selected: None,
            context_select,
            namespace_select,
            kind_select,
            search,
            query: String::new(),
            loading: false,
            problem: None,
            generation: 0,
            task: None,
            details: None,
            scroll: UniformListScrollHandle::new(),
            dispatch,
            focus: cx.focus_handle(),
            window: window.window_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_target(&mut self, target: Option<ExecTarget>, active: bool, cx: &mut Context<Self>) {
        if self.target == target && self.active == active {
            return;
        }
        if self.target != target {
            self.context = None;
            self.contexts.clear();
            self.namespace = None;
            self.namespaces.clear();
            self.namespaces_ready = false;
            self.rows = Rc::new(Vec::new());
            self.selected = None;
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            self.details = None;
            let entity = cx.entity().downgrade();
            let window_handle = self.window;
            cx.defer(move |cx| {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| this.sync_selects(window, cx));
                });
            });
        }
        self.target = target;
        self.active = active;
        self.read(true, cx);
        cx.notify();
    }

    pub fn command(
        &mut self,
        command: &KubernetesCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            KubernetesCommand::Refresh => self.read(true, cx),
            KubernetesCommand::Context(context) if self.contexts.contains(context) => {
                if self.context.as_ref() != Some(context) {
                    self.context = Some(context.clone());
                    self.namespace = None;
                    self.namespaces.clear();
                    self.namespaces_ready = false;
                    self.rows = Rc::new(Vec::new());
                    self.selected = None;
                    self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    self.read(true, cx);
                }
            }
            KubernetesCommand::Namespace(namespace)
                if namespace
                    .as_ref()
                    .is_none_or(|ns| self.namespaces.contains(ns)) =>
            {
                if &self.namespace != namespace {
                    self.namespace = namespace.clone();
                    self.rows = Rc::new(Vec::new());
                    self.selected = None;
                    self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    self.read(false, cx);
                }
            }
            KubernetesCommand::Kind(kind) => {
                if self.kind != *kind {
                    self.kind = *kind;
                    self.rows = Rc::new(Vec::new());
                    self.selected = None;
                    self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    self.read(false, cx);
                }
            }
            KubernetesCommand::Details { namespace, name } => {
                if let (Some(target), Some(context), Some(resource)) = (
                    self.target.clone(),
                    self.context.clone(),
                    self.rows
                        .iter()
                        .find(|row| &row.namespace == namespace && &row.name == name)
                        .cloned(),
                ) {
                    self.selected = Some(resource.key());
                    let view = cx.new(|cx| {
                        KubernetesDetails::new(
                            resource.clone(),
                            context.clone(),
                            target,
                            self.dispatch.clone(),
                            window,
                            cx,
                        )
                    });
                    self.details = Some(view.downgrade());
                    let title = format!("Kubernetes · {context}");
                    let dispatch = self.dispatch.clone();
                    let focus = self.focus.clone();
                    window.open_dialog(cx, move |dialog, window, _| {
                        let dispatch = dispatch.clone();
                        let focus = focus.clone();
                        dialog
                            .title(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                                    .child(
                                        Button::new("k8s-detail-close")
                                            .ghost()
                                            .small()
                                            .icon(IconName::Close)
                                            .accessibility_label(t!("common.close"))
                                            .tooltip(t!("common.close"))
                                            .on_click(move |_, window, cx| {
                                                // The default DialogClose dispatches Cancel
                                                // through current focus, which may be outside
                                                // this dialog after terminal activation.
                                                defer_dispatch(
                                                    &dispatch,
                                                    &KubernetesAction(
                                                        KubernetesCommand::CloseDetails,
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                    ),
                            )
                            .w(rems(58.).to_pixels(window.rem_size()))
                            .max_h(window.viewport_size().height * 0.9)
                            .close_button(false)
                            .overlay_closable(true)
                            .on_close(move |_, window, cx| focus.focus(window, cx))
                            .child(view.clone())
                    });
                }
            }
            KubernetesCommand::CloseDetails => {
                if self
                    .details
                    .take()
                    .and_then(|details| details.upgrade())
                    .is_some()
                {
                    window.close_dialog(cx);
                    self.focus.focus(window, cx);
                }
            }
            command => {
                if let Some(details) = self.details.as_ref().and_then(WeakEntity::upgrade) {
                    details.update(cx, |details, cx| details.command(command, window, cx));
                }
            }
        }
        self.sync_selects(window, cx);
        cx.notify();
    }

    fn sync_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_select.update(cx, |state, cx| {
            state.set_items(
                self.contexts
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect(),
                window,
                cx,
            );
            let ix = self
                .contexts
                .iter()
                .position(|name| Some(name) == self.context.as_ref());
            state.set_selected_index(ix.map(IndexPath::new), window, cx);
        });
        self.namespace_select.update(cx, |state, cx| {
            let mut names = vec![t!("k8s.all")];
            names.extend(self.namespaces.iter().cloned().map(SharedString::from));
            state.set_items(names, window, cx);
            let ix = self
                .namespace
                .as_ref()
                .and_then(|ns| self.namespaces.iter().position(|name| name == ns))
                .map_or(0, |ix| ix + 1);
            state.set_selected_index(Some(IndexPath::new(ix)), window, cx);
        });
        self.kind_select.update(cx, |state, cx| {
            state.set_selected_index(
                ResourceKind::ALL
                    .iter()
                    .position(|kind| *kind == self.kind)
                    .map(IndexPath::new),
                window,
                cx,
            )
        });
    }

    fn read(&mut self, discover: bool, cx: &mut Context<Self>) {
        // Changing resource type can cancel discovery between context and
        // namespace replies. Finish discovering namespaces before any list.
        let discover = discover || !self.namespaces_ready;
        self.generation += 1;
        self.task = None;
        self.loading = false;
        let Some(target) = self.target.clone().filter(|_| self.active) else {
            return;
        };
        self.loading = true;
        self.problem = None;
        let (generation, window, context, namespace, kind) = (
            self.generation,
            self.window,
            self.context.clone(),
            self.namespace.clone(),
            self.kind,
        );
        self.task = Some(cx.spawn(async move |this, cx| {
            // Wait for a terminal still establishing its connection. Do not
            // retry RBAC, API or kubectl errors without an explicit refresh.
            let reply = loop {
                let command = if discover || context.is_none() {
                    commands::contexts_command()
                } else {
                    commands::list(
                        context.as_deref().unwrap_or_default(),
                        namespace.as_deref(),
                        kind,
                        None,
                    )
                };
                let Ok(reply) = this.update(cx, |_, cx| {
                    target.view.upgrade()?.read(cx).exec(command, cx)
                }) else {
                    return;
                };
                if let Some(reply) = reply {
                    break reply;
                }
                let _ = this.update(cx, |this, cx| {
                    this.problem = Some(t!("tools.not_connected").to_string());
                    cx.notify();
                });
                cx.background_executor().timer(Duration::from_secs(2)).await;
            };
            let first = exec_answer(reply, cx)
                .await
                .unwrap_or_else(|| Err(t!("tools.not_connected").to_string()));
            let chosen = context;
            let mut selected_namespace = namespace;
            let result = async {
                let first = first?;
                if discover || chosen.is_none() {
                    let (current, contexts) = commands::contexts(&first)?;
                    let name = chosen
                        .filter(|name| contexts.contains(name))
                        .unwrap_or(current);
                    let _ = cx.update_window(window, |_, window, cx| {
                        this.update(cx, |this, cx| {
                            if this.generation == generation {
                                if this.context.as_ref() != Some(&name) {
                                    this.rows = Rc::new(Vec::new());
                                }
                                this.context = Some(name.clone());
                                this.contexts = contexts;
                                this.sync_selects(window, cx);
                                cx.notify();
                            }
                        })
                    });
                    let namespaces = request(&target, commands::namespaces(&name), cx).await?;
                    let namespaces = model::names(&commands::answer(&namespaces)?)?;
                    selected_namespace = selected_namespace.filter(|ns| namespaces.contains(ns));
                    let _ = cx.update_window(window, |_, window, cx| {
                        this.update(cx, |this, cx| {
                            if this.generation == generation {
                                this.namespace = selected_namespace.clone();
                                this.namespaces = namespaces;
                                this.namespaces_ready = true;
                                this.sync_selects(window, cx);
                                cx.notify();
                            }
                        })
                    });
                    let output = request(
                        &target,
                        commands::list(&name, selected_namespace.as_deref(), kind, None),
                        cx,
                    )
                    .await?;
                    model::resources(&commands::answer(&output)?, kind)
                } else {
                    model::resources(&commands::answer(&first)?, kind)
                }
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(rows) => this.rows = Rc::new(rows),
                    Err(why) => this.problem = Some(why),
                }
                cx.notify();
            });
        }));
    }
}

pub(super) async fn request(
    target: &ExecTarget,
    command: String,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let reply = cx
        .update(|cx| {
            target
                .view
                .upgrade()
                .and_then(|view| view.read(cx).exec(command, cx))
        })
        .ok_or_else(|| t!("tools.not_connected").to_string())?;
    exec_answer(reply, cx)
        .await
        .unwrap_or_else(|| Err(t!("tools.not_connected").to_string()))
}

impl Focusable for KubernetesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for KubernetesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let visible = Rc::new(
            rows.iter()
                .enumerate()
                .filter(|(_, row)| row.matches(&self.query))
                .map(|(ix, _)| ix)
                .collect::<Vec<_>>(),
        );
        let refresh = self.dispatch.clone();
        let dispatch = self.dispatch.clone();
        let selected = self.selected.clone();
        let count = t!(
            "k8s.count",
            count = visible.len(),
            namespaces = self.namespaces.len()
        );
        v_flex()
            .id("kubernetes")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .flex_shrink_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().text_sm().child(t!("k8s.context")))
                            .child(
                                Button::new("k8s-refresh")
                                    .ghost()
                                    .small()
                                    .label(t!("tools.refresh"))
                                    .loading(self.loading)
                                    .on_click(move |_, window, cx| {
                                        defer_dispatch(
                                            &refresh,
                                            &KubernetesAction(KubernetesCommand::Refresh),
                                            window,
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .child(
                        Select::new(&self.context_select)
                            .id("k8s-context")
                            .small()
                            .disabled(self.contexts.is_empty()),
                    )
                    .child(div().text_sm().child(t!("k8s.namespace")))
                    .child(
                        Select::new(&self.namespace_select)
                            .id("k8s-namespace")
                            .small()
                            .disabled(self.context.is_none()),
                    )
                    .child(div().text_sm().child(t!("k8s.kind")))
                    .child(Select::new(&self.kind_select).id("k8s-kind").small())
                    .child(Input::new(&self.search).id("k8s-search").small())
                    .child(
                        div()
                            .id("k8s-count")
                            .test_support()
                            .aria_label(count.clone())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count),
                    )
                    .when(self.loading && !self.rows.is_empty(), |header| {
                        header.child(reading("k8s-reading", cx))
                    })
                    .when_some(self.problem.clone(), |header, why| {
                        header.child(
                            div()
                                .id("k8s-error")
                                .test_support()
                                .aria_label(why.clone())
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(t!("k8s.failed", error = why)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .map(|body| {
                        if visible.is_empty() && self.problem.is_some() && !self.loading {
                            body
                        } else if visible.is_empty() {
                            if self.loading {
                                body.child(reading("k8s-list-reading", cx))
                            } else {
                                body.child(
                                    div()
                                        .p_3()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("k8s.empty")),
                                )
                            }
                        } else {
                            body.child(
                                uniform_list(
                                    "k8s-resources",
                                    visible.len(),
                                    move |range, _, cx| {
                                        range
                                            .map(|ix| {
                                                let resource = &rows[visible[ix]];
                                                let action =
                                                    KubernetesAction(KubernetesCommand::Details {
                                                        namespace: resource.namespace.clone(),
                                                        name: resource.name.clone(),
                                                    });
                                                let handle = dispatch.clone();
                                                v_flex()
                                                    .id(SharedString::from(resource.key()))
                                                    .test_support()
                                                    .aria_label(format!(
                                                        "{}/{} · {}",
                                                        resource.namespace,
                                                        resource.name,
                                                        resource.summary
                                                    ))
                                                    .px_3()
                                                    .py_1()
                                                    .gap_1()
                                                    .child(
                                                        h_flex()
                                                            .gap_1()
                                                            .child(
                                                                Button::new(SharedString::from(
                                                                    format!(
                                                                        "{}:open",
                                                                        resource.key()
                                                                    ),
                                                                ))
                                                                .ghost()
                                                                .selected(
                                                                    selected.as_deref()
                                                                        == Some(
                                                                            resource.key().as_str(),
                                                                        ),
                                                                )
                                                                .small()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .justify_start()
                                                                .label(resource.name.clone())
                                                                .tooltip(t!("k8s.open_details"))
                                                                .on_click(move |_, window, cx| {
                                                                    defer_dispatch(
                                                                        &handle, &action, window,
                                                                        cx,
                                                                    )
                                                                }),
                                                            )
                                                            .child(copy_button(
                                                                SharedString::from(format!(
                                                                    "{}:copy",
                                                                    resource.key()
                                                                )),
                                                                resource.name.clone(),
                                                                &dispatch,
                                                            )),
                                                    )
                                                    .when_some(
                                                        resource.status.clone(),
                                                        |row, (label, tone)| {
                                                            row.child(h_flex().child(soft_tag(
                                                                label,
                                                                tone_color(tone, cx),
                                                            )))
                                                        },
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .truncate()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(format!(
                                                                "{} · {}",
                                                                resource.namespace,
                                                                resource.summary
                                                            )),
                                                    )
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                )
                                .size_full()
                                .track_scroll(&self.scroll),
                            )
                        }
                    })
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

// Select subscriptions run inside the panel update. Route commands only
// after that update ends, including clicks originating in a dialog.
pub(super) fn defer_dispatch<A: Action + Clone>(
    target: &FocusHandle,
    action: &A,
    window: &mut Window,
    cx: &mut App,
) {
    let target = target.clone();
    let action = action.clone();
    window.defer(cx, move |window, cx| {
        target.dispatch_action(&action, window, cx)
    });
}
