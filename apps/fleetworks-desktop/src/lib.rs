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
pub mod configuration;
mod configuration_views;
pub mod connection;
pub mod connection_profiles;
pub mod icons;
pub mod job_actions;
mod jobs_effects;
pub mod jobs_query;
pub mod launch;
mod log_effects;
pub mod logs;
pub mod maintenance;
pub mod maintenance_result;
mod material;
pub mod pairing;
mod platform;
mod platform_trash;
mod readiness_cards;
pub mod rebind;
pub mod registration;
pub mod submission;
pub mod theme;
pub mod transfer;
pub mod ui_components;
mod views;
mod workspace_lifecycle;
pub mod workspaces;
pub mod workstation_dashboard;

pub use app::Page;
pub use app::State;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
