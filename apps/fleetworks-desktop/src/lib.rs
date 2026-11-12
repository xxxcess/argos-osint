//! API-only Fleetworks native presentation and boundary effects.
//!
//! Production and development fixtures share this shell, state, views and handlers.

#![cfg(target_os = "macos")]

pub mod accessibility;
pub mod app;
pub mod backend;
mod backend_views;
pub mod brain;
pub mod classifier_activation;
mod classifier_activation_views;
pub mod configuration;
mod configuration_views;
pub mod connection;
mod job_action_views;
pub mod job_actions;
pub mod launch;
pub mod maintenance;
pub mod maintenance_result;
mod material;
mod platform;
mod platform_trash;
mod readiness_cards;
pub mod rebind;
mod rebind_views;
pub mod registration;
pub mod submission;
mod submission_views;
pub mod theme;
pub mod transfer;
mod views;
mod workspace_lifecycle;
pub mod workspaces;

pub use app::Page;
pub use app::State;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
