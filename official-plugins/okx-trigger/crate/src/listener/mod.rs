use std::io::{self, Write};

use crate::contract::{ready_message, TriggerStartCommand};

pub mod market_stream;
pub mod normalize;
pub mod policy;
pub mod private_stream;
pub mod types;

pub async fn run_listener(command: &TriggerStartCommand) -> Result<(), String> {
    let mut stdout = io::stdout();
    run_listener_with_writer(command, &mut stdout).await
}

pub async fn run_listener_with_writer(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    match command.source.as_str() {
        "okx_market_stream" | "okx_private_stream" => {}
        other => return Err(format!("unsupported OKX trigger source {other}")),
    }

    if command
        .params
        .get("endpoint")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|endpoint| endpoint.starts_with("mock://"))
    {
        write_ready(stdout)?;
        return normalize::emit_mock_event(command, stdout);
    }

    match command.source.as_str() {
        "okx_market_stream" => market_stream::run_market_listener(command, stdout).await,
        "okx_private_stream" => private_stream::run_private_listener(command, stdout).await,
        _ => unreachable!(),
    }
}

pub fn write_ready(stdout: &mut impl Write) -> Result<(), String> {
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}
