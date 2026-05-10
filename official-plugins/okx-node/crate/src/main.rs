use std::io::{self, Read};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        println!("{}", okx_node_official_plugin::failure_response("failed to read stdin"));
        return;
    }

    match okx_node_official_plugin::handle_request_json(&input).await {
        Ok(response) => println!("{response}"),
        Err(error) => println!(
            "{}",
            okx_node_official_plugin::failure_response(&error.to_string())
        ),
    }
}
