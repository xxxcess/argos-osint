//! API-only Fleetworks native presentation and boundary effects.
//!
//! Production and development fixtures share this shell, state, views and handlers.

#![cfg(any(target_os = "macos", target_os = "linux"))]

pub mod accessibility;
pub mod app;
pub mod assistant;
pub mod backend;
mod backend_views;
pub mod brain;
mod catalog;
mod chat_ui;
pub mod classifier_activation;
pub mod command_panel;
pub mod configuration;
mod configuration_views;
pub mod connection;
pub mod connection_profiles;
pub mod icons;
pub mod inbox;
pub mod job_actions;
mod jobs_effects;
pub mod jobs_query;
pub mod launch;
pub mod local_backend;
mod log_effects;
pub mod logs;
pub mod maintenance;
pub mod maintenance_result;
mod material;
mod orbit;
pub mod pairing;
mod platform;
mod platform_trash;
mod readiness_cards;
pub mod rebind;
pub mod registration;
mod rock;
pub mod startup;
mod startup_checks;
mod startup_data;
pub mod submission;
pub mod theme;
pub mod transfer;
pub mod ui_components;
mod views;
mod workspace_lifecycle;
pub mod workspaces;
pub mod workstation_dashboard;
mod workstation_settings;

pub use app::Page;
pub use app::State;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
