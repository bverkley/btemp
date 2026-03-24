//! Terminal UI for live temperature graphs from lm-sensors.

fn main() -> anyhow::Result<()> {
    btemp::run()
}
