#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    fleetworks_desktop::launch::run()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "Fleetworks Desktop requires macOS or Linux. Run the independent Fleetworks services on this host."
    )
}
