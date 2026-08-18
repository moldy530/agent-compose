//! `mock-provider` — the acceptance harness as a standalone process.
//!
//! ```text
//! mock-provider [--host <ip>] [--port <n>]
//! ```
//!
//! The library form ([`mock_provider::MockProvider`]) is what a Rust test uses;
//! this is what everything else uses — the TypeScript e2e harness M1 will grow,
//! and a person poking at a compiled graph by hand. It binds, prints one JSON
//! line naming the address, and serves until it is killed.
//!
//! The line is printed and flushed **before** the first request can arrive, so a
//! harness that starts this process can read one line and know the server is
//! ready. `--port 0` (the default) takes an ephemeral port, which is what makes
//! parallel runs safe; the line is how the caller learns which one.

use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;

use clap::Parser;
use mock_provider::MockProvider;

#[derive(Parser)]
#[command(name = "mock-provider", version, about)]
struct Cli {
    /// Address to bind; loopback by default, because a scripted provider is not
    /// something to expose
    #[arg(long, value_name = "IP", default_value = "127.0.0.1")]
    host: IpAddr,
    /// Port to bind; `0` takes an ephemeral one and prints it
    #[arg(long, value_name = "PORT", default_value_t = 0)]
    port: u16,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let provider = match MockProvider::start_on(SocketAddr::new(cli.host, cli.port)) {
        Ok(provider) => provider,
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "error: cannot bind {}:{}: {error}",
                cli.host,
                cli.port
            );
            return ExitCode::from(2);
        }
    };

    let ready = serde_json::json!({
        "address": provider.address().to_string(),
        "base_url": provider.base_url(),
    });
    let mut stdout = io::stdout().lock();
    if writeln!(stdout, "{ready}")
        .and_then(|()| stdout.flush())
        .is_err()
    {
        // Nobody is reading, so nobody is going to script anything either.
        return ExitCode::from(2);
    }
    drop(stdout);

    // The server runs on the handle's own runtime; this thread has nothing left
    // to do but stay alive so the handle is not dropped.
    loop {
        std::thread::park();
    }
}
