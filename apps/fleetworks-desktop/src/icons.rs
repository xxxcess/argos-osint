//! Custom crisp 16px SVG glyphs tailored to Fleetworks destination and telemetry semantics.

use rune::Icon;

/// Fleetworks interlocking diamond emblem.
pub static BRAND_MARK: Icon = Icon {
    name: "fw-brand-mark",
    view: 16.0,
    body: r#"<path d="M8 1.5 14 5v6l-6 3.5L2 11V5l6-3.5Z" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M8 5 11.2 6.8v3.6L8 12.2 4.8 10.4V6.8L8 5Z" fill="currentColor" opacity="0.88"/><path d="M8 1.5V5M14 5l-2.8 1.8M2 5l2.8 1.8M8 12.2v2.3" stroke="currentColor" stroke-width="1.2"/>"#,
};

/// Workstation telemetry chassis.
pub static WORKSTATION: Icon = Icon {
    name: "fw-workstation",
    view: 16.0,
    body: r#"<rect x="2" y="2.5" width="12" height="9" rx="1.6" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M3.5 7.5h2.2l1.2-2.2 1.8 4 1.3-1.8h2.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M5.5 13.5h5M8 11.5v2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};

/// Seed & classifier stacked database strata.
pub static SEED: Icon = Icon {
    name: "fw-seed",
    view: 16.0,
    body: r#"<ellipse cx="8" cy="4" rx="5.2" ry="2" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M2.8 4v4c0 1.1 2.3 2 5.2 2s5.2-.9 5.2-2V4" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M2.8 8v4c0 1.1 2.3 2 5.2 2s5.2-.9 5.2-2V8" stroke="currentColor" stroke-width="1.4" fill="none"/>"#,
};

/// News cycles & collection sweep radar.
pub static ATLAS: Icon = Icon {
    name: "fw-atlas",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.4" fill="none"/><circle cx="8" cy="8" r="2.6" stroke="currentColor" stroke-width="1.2" opacity="0.65" fill="none"/><path d="M8 8 12.2 3.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><circle cx="8" cy="8" r="1.2" fill="currentColor"/>"#,
};

/// Brain intelligence node lattice.
pub static BRAIN: Icon = Icon {
    name: "fw-brain",
    view: 16.0,
    body: r#"<circle cx="8" cy="3.2" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="3.4" cy="11.2" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="12.6" cy="11.2" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M7.1 4.6 4.3 9.7M8.9 4.6l2.8 5.1M5.1 11.2h5.8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};

/// OSINT tool connectors & keys.
pub static TOOLS: Icon = Icon {
    name: "fw-tools",
    view: 16.0,
    body: r#"<path d="M10.8 2.6a3.2 3.2 0 0 0-4.1 4.1l-4 4a1.2 1.2 0 0 0 1.7 1.7l4-4a3.2 3.2 0 0 0 4.1-4.1l-1.6 1.6-1.5-.3-.3-1.5 1.7-1.5Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" fill="none"/>"#,
};

/// Model routing layers.
pub static MODELS: Icon = Icon {
    name: "fw-models",
    view: 16.0,
    body: r#"<path d="M8 2.2 13.8 5.2 8 8.2 2.2 5.2 8 2.2Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" fill="none"/><path d="M2.2 8 8 11l5.8-3M2.2 10.8 8 13.8l5.8-3" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Jobs & activity pulse waveform.
pub static JOBS: Icon = Icon {
    name: "fw-jobs",
    view: 16.0,
    body: r#"<rect x="2" y="2.5" width="12" height="11" rx="1.8" stroke="currentColor" stroke-width="1.35" fill="none"/><path d="M4.2 8.2h2l1.2-2.6 1.6 4.8 1.2-2.2h1.6" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Processor / CPU core.
pub static CPU: Icon = Icon {
    name: "fw-cpu",
    view: 16.0,
    body: r#"<rect x="4.5" y="4.5" width="7" height="7" rx="1" stroke="currentColor" stroke-width="1.35" fill="none"/><rect x="6.5" y="6.5" width="3" height="3" fill="currentColor" opacity="0.8"/><path d="M6.5 2v2.5M9.5 2v2.5M6.5 11.5V14M9.5 11.5V14M2 6.5h2.5M2 9.5h2.5M11.5 6.5H14M11.5 9.5H14" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};

/// Memory module / RAM bank.
pub static MEMORY: Icon = Icon {
    name: "fw-memory",
    view: 16.0,
    body: r#"<rect x="2" y="4" width="12" height="6.5" rx="1.2" stroke="currentColor" stroke-width="1.35" fill="none"/><path d="M4.8 6.2v2.1M8 6.2v2.1M11.2 6.2v2.1M4.5 10.5V13M8 10.5V13M11.5 10.5V13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};

/// Disk storage platter.
pub static DISK: Icon = Icon {
    name: "fw-disk",
    view: 16.0,
    body: r#"<rect x="2.5" y="3" width="11" height="10" rx="1.6" stroke="currentColor" stroke-width="1.35" fill="none"/><path d="M2.5 9.8h11" stroke="currentColor" stroke-width="1.3"/><circle cx="10.8" cy="11.4" r="0.8" fill="currentColor"/>"#,
};

/// Credential key icon.
pub static KEY: Icon = Icon {
    name: "fw-key",
    view: 16.0,
    body: r#"<circle cx="5.2" cy="8" r="2.6" stroke="currentColor" stroke-width="1.35" fill="none"/><path d="M7.8 8H14M11.5 8v2.2M13.5 8v1.6" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"/>"#,
};

/// Shielded verification / readiness check.
pub static SHIELD_CHECK: Icon = Icon {
    name: "fw-shield-check",
    view: 16.0,
    body: r#"<path d="M8 2 13 4v3.8c0 3.2-2.1 5.4-5 6.2-2.9-.8-5-3-5-6.2V4l5-2Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" fill="none"/><path d="M5.6 8.1 7.2 9.7l3.3-3.3" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Warning or fault triangle.
pub static ALERT: Icon = Icon {
    name: "fw-alert",
    view: 16.0,
    body: r#"<path d="M8 2.4 13.8 13H2.2L8 2.4Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" fill="none"/><path d="M8 6.4v3.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><circle cx="8" cy="11.3" r="0.75" fill="currentColor"/>"#,
};

/// Gateway server node.
pub static SERVER: Icon = Icon {
    name: "fw-server",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="11" height="4.5" rx="1.2" stroke="currentColor" stroke-width="1.35" fill="none"/><rect x="2.5" y="9" width="11" height="4.5" rx="1.2" stroke="currentColor" stroke-width="1.35" fill="none"/><circle cx="5" cy="4.75" r="0.75" fill="currentColor"/><circle cx="5" cy="11.25" r="0.75" fill="currentColor"/>"#,
};

/// Refresh / sync arrows.
pub static REFRESH: Icon = Icon {
    name: "fw-refresh",
    view: 16.0,
    body: r#"<path d="M13 7.5A5 5 0 0 0 4.3 4.6L3 6M3 2.8V6h3.2" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M3 8.5a5 5 0 0 0 8.7 2.9L13 10m0 3.2V10H9.8" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Trash / delete glyph.
pub static TRASH: Icon = Icon {
    name: "fw-trash",
    view: 16.0,
    body: r#"<path d="M3 4.5h10M6.2 4.5V3h3.6v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Chevron right indicator for drilldown cards.
pub static CHEVRON_RIGHT: Icon = Icon {
    name: "fw-chevron-right",
    view: 16.0,
    body: r#"<path d="M6 3.5 10.5 8 6 12.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Plus / add glyph.
pub static PLUS: Icon = Icon {
    name: "fw-plus",
    view: 16.0,
    body: r#"<path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>"#,
};
