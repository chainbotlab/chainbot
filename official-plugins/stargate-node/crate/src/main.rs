use std::io::{self, Read};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        println!("{}", stargate_node_official_plugin::failure_response("failed to read stdin"));
        return;
    }
    let response = match stargate_node_official_plugin::handle_request_json(&input).await {
        Ok(response) => response,
        Err(error) => stargate_node_official_plugin::failure_response(&error.to_string()),
    };
    println!("{response}");
}
