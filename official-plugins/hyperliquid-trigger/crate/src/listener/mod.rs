mod market_stream;
mod normalize;
mod policy;

use std::io::{self, Write};

use serde_json::Value;

use crate::contract::{ready_message, TriggerStartCommand};

pub async fn run_listener(command: TriggerStartCommand) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    run_listener_with_writer(command, &mut stdout).await
}

pub async fn run_listener_with_writer(command: TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let endpoint_override = command.params.get("endpoint").and_then(Value::as_str);
    if matches!(endpoint_override, Some(value) if value.starts_with("mock://")) {
        write_ready(stdout)?;
        return normalize::emit_mock_event(&command, stdout);
    }

    market_stream::run_market_listener(&command, stdout).await
}

pub(crate) fn write_ready(stdout: &mut impl Write) -> Result<(), String> {
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}
