use std::io::{self, BufRead};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return;
    }
    watch_host_control();

    if let Err(error) = solana_trigger_official_plugin::run_from_stdin(&input).await {
        eprintln!("{error}");
    }
}

fn watch_host_control() {
    std::thread::spawn(|| {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) if line.contains("\"type\":\"stop\"") => {
                    std::process::exit(0);
                }
                Ok(_) => {}
                Err(_) => std::process::exit(0),
            }
        }
        std::process::exit(0);
    });
}
