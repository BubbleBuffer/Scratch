//! The schedule format for this crate, which is `crates/agent/src/schedules/format.rs`
//! itself. Desktop re-exports vault_agent's copy. Every other target compiles the same
//! file here, so schedule preview needs no desktop agent runtime. Tests compile the
//! portable copy on any host too, so it is exercised wherever the tests run.

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) use vault_agent::schedules::format;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[allow(dead_code)]
#[path = "../../../crates/agent/src/schedules/format.rs"]
pub(crate) mod format;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../crates/agent/src/schedules/format.rs"]
pub(crate) mod portable;
