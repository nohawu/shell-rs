use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    spinner::Spinner,
};
use gpui_kit::*;

use super::{
    model::{KubernetesCommand, Tone},
    panel::defer_dispatch,
};
use crate::{app::KubernetesAction, i18n::t};

pub(super) fn tone_color(tone: Tone, cx: &App) -> Hsla {
    match tone {
        Tone::Good => cx.theme().success,
        Tone::Warning => cx.theme().warning,
        Tone::Danger => cx.theme().danger,
        Tone::Quiet => cx.theme().muted_foreground,
    }
}

pub(super) fn reading(id: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .id(id)
        .test_support()
        .gap_2()
        .justify_center()
        .py_3()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Spinner::new().small())
        .child(t!("tools.reading"))
        .into_any_element()
}

pub(super) fn copy_button(
    id: impl Into<ElementId>,
    value: String,
    dispatch: &FocusHandle,
) -> Button {
    let handle = dispatch.clone();
    Button::new(id)
        .ghost()
        .small()
        .icon(IconName::Copy)
        .tooltip(t!("history.menu.copy"))
        .on_click(move |_, window, cx| {
            defer_dispatch(
                &handle,
                &KubernetesAction(KubernetesCommand::CopyValue(value.clone())),
                window,
                cx,
            )
        })
}
