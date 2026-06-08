use alloy::primitives::U256;
use uniswap_node_official_plugin::provider::{
    encode_get_amounts_out, encode_swap_exact_tokens_for_tokens, parse_address,
};

#[test]
fn get_amounts_out_calldata_matches_router02_abi_layout() {
    let token_a = parse_address("0x0000000000000000000000000000000000000002", "token_a")
        .expect("token_a should parse");
    let token_b = parse_address("0x0000000000000000000000000000000000000003", "token_b")
        .expect("token_b should parse");

    let calldata = encode_get_amounts_out(U256::from(100), &[token_a, token_b]);
    let hex = hex::encode(calldata.as_ref());

    assert!(hex.starts_with("d06de04f"));
    assert_eq!(&hex[8..72], &word(100));
    assert_eq!(&hex[72..136], &word(64));
    assert_eq!(&hex[136..200], &word(2));
    assert_eq!(
        &hex[200..264],
        "0000000000000000000000000000000000000000000000000000000000000002"
    );
    assert_eq!(
        &hex[264..328],
        "0000000000000000000000000000000000000000000000000000000000000003"
    );
}

#[test]
fn swap_exact_tokens_for_tokens_calldata_has_dynamic_path_offset() {
    let token_a = parse_address("0x0000000000000000000000000000000000000002", "token_a")
        .expect("token_a should parse");
    let token_b = parse_address("0x0000000000000000000000000000000000000003", "token_b")
        .expect("token_b should parse");
    let recipient = parse_address("0x0000000000000000000000000000000000000004", "recipient")
        .expect("recipient should parse");

    let calldata = encode_swap_exact_tokens_for_tokens(
        U256::from(100),
        U256::from(90),
        &[token_a, token_b],
        recipient,
        U256::from(1_800_000_000u64),
    );
    let hex = hex::encode(calldata.as_ref());

    assert!(hex.starts_with("38ed1739"));
    assert_eq!(&hex[8..72], &word(100));
    assert_eq!(&hex[72..136], &word(90));
    assert_eq!(&hex[136..200], &word(160));
    assert_eq!(
        &hex[200..264],
        "0000000000000000000000000000000000000000000000000000000000000004"
    );
    assert_eq!(&hex[264..328], &word(1_800_000_000));
    assert_eq!(&hex[328..392], &word(2));
}

fn word(value: u64) -> String {
    format!("{value:064x}")
}
