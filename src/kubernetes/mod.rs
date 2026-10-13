//! Kubernetes resources and container logs, using kubectl on the SSH host.
//! No kubeconfig or cluster credentials are copied into ShellRS.

mod commands;
mod details;
mod logs;
mod model;
mod panel;
mod presentation;

pub use model::{KubernetesCommand, ResourceKind};
pub use panel::KubernetesPanel;

#[cfg(test)]
mod tests;
