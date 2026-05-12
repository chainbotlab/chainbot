use std::io::{self, Read};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("failed to read stdin: {error}");
        std::process::exit(1);
    }
    if let Err(error) = okx_trigger_official_plugin::run_from_stdin(&input).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
