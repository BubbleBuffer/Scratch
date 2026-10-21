use flutter_rust_bridge::frb;

/// This device's IANA time-zone name for new schedule definitions.
#[frb(sync)]
pub fn device_time_zone() -> Result<String, String> {
    let zone = jiff::tz::TimeZone::try_system().map_err(|error| error.to_string())?;
    zone.iana_name()
        .map(str::to_owned)
        .ok_or_else(|| "the system time zone has no IANA name".to_owned())
}

/// Test-only process clock. Set before opening vaults; None restores real time.
#[frb(sync)]
pub fn set_test_clock(start_ms: Option<i64>) {
    vault_core::Vault::set_test_clock(start_ms);
}
