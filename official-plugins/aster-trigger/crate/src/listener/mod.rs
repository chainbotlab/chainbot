mod market_stream;
mod normalize;
mod policy;
mod types;

use std::io::{self, Write};

use serde_json::Value;

use crate::contract::{ready_message, TriggerStartCommand};

pub async fn run_listener(command: TriggerStartCommand) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    run_listener_with_writer(command, &mut stdout).await
}

pub async fn run_listener_with_writer(
    command: TriggerStartCommand,
    stdout: &mut impl Write,
) -> Result<(), String> {
    let source = command.source.as_str();
    if source != "aster_market_stream" {
        return Err(format!("unsupported Aster trigger source {source}"));
    }

    let endpoint_override = command.params.get("endpoint").and_then(Value::as_str);
    if matches!(endpoint_override, Some(value) if value.starts_with("mock://")) {
        write_ready(stdout)?;
        return normalize::emit_mock_event(&command, stdout);
    }

    match source {
        "aster_market_stream" => market_stream::run_market_listener(&command, stdout).await,
        other => Err(format!("unsupported Aster trigger source {other}")),
    }
}

pub(crate) fn write_ready(stdout: &mut impl Write) -> Result<(), String> {
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}
