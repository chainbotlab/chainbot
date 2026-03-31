use std::io::{self, Read};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }

    let _ = solana_trigger_official_plugin::run_from_stdin(&input).await;
}
