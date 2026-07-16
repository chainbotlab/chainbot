use std::io::{self, BufRead};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return;
    }
    watch_host_control();

    let _ = solana_trigger_official_plugin::run_from_stdin(&input).await;
}

fn watch_host_control() {
    std::thread::spawn(|| {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if line.contains("\"type\":\"stop\"") {
                std::process::exit(0);
            }
        }
    });
}
