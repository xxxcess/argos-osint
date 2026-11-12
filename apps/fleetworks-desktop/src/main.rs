#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    fleetworks_desktop::launch::run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "Fleetworks Desktop requires macOS. Run the independent Fleetworks services on this host."
    )
}
