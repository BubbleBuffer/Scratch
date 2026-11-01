//! `vault-hub <vault-dir> --listen <addr>`: serves a vault to paired
//! devices. `--pair <label>` prints a one-time pairing code (10 minutes) for a
//! device, redeemed at `POST /auth/pair`; `--unpair <label>`
//! revokes it; `--devices` lists paired devices.

use std::sync::Arc;

use vault_core::Vault;
use vault_sync::Hub;

const USAGE: &str = "usage:
  vault-hub <vault-dir> --listen <addr>     serve the vault (for example --listen 100.101.102.103:8787)
  vault-hub <vault-dir> --pair <label>      create a pairing code for a device
  vault-hub <vault-dir> --unpair <label>    revoke a device's token
  vault-hub <vault-dir> --devices           list paired devices";

fn main() {
    if let Err(e) = run() {
        eprintln!("vault-hub: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, cmd, arg) = match args.as_slice() {
        [dir, cmd] => (dir, cmd.as_str(), None),
        [dir, cmd, arg] => (dir, cmd.as_str(), Some(arg.as_str())),
        _ => return Err(USAGE.into()),
    };
    let vault = Arc::new(Vault::open(dir.clone()).map_err(|e| e.to_string())?);
    let hub = Hub::new(vault).map_err(|e| e.to_string())?;
    match (cmd, arg) {
        ("--listen", Some(addr)) => {
            let server = hub.serve(addr).map_err(|e| e.to_string())?;
            println!("vault-hub: serving {dir} on http://{}", server.addr());
            loop {
                std::thread::park();
            }
        }
        ("--pair", Some(label)) => {
            let code = hub.pair(label).map_err(|e| e.to_string())?;
            println!("{code}");
            Ok(())
        }
        ("--unpair", Some(label)) => {
            let n = hub.unpair(label).map_err(|e| e.to_string())?;
            println!("revoked {n} token(s)");
            Ok(())
        }
        ("--devices", None) => {
            for d in hub.devices() {
                println!(
                    "{}\t{}",
                    d.label,
                    d.device.as_deref().unwrap_or("(not used yet)")
                );
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
