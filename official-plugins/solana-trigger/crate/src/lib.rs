pub mod contract;
pub mod listener;

pub async fn run_from_stdin(input: &str) -> Result<(), String> {
    let command = contract::parse_start_command(input)?;
    listener::run_listener(command).await
}
