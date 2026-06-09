use alloy::primitives::U256;
use pancakeswap_node_official_plugin::provider::{
    bytes_from_hex, encode_exact_input, encode_exact_input_single, parse_address,
};

#[test]
fn exact_input_single_calldata_matches_pancakeswap_v3_layout() {
    let token_in = parse_address("0x0000000000000000000000000000000000000002", "token_in")
        .expect("token_in should parse");
    let token_out = parse_address("0x0000000000000000000000000000000000000003", "token_out")
        .expect("token_out should parse");
    let recipient = parse_address("0x0000000000000000000000000000000000000004", "recipient")
        .expect("recipient should parse");

    let calldata = encode_exact_input_single(
        token_in,
        token_out,
        2500,
        recipient,
        U256::from(1_800_000_000u64),
        U256::from(100),
        U256::from(90),
        U256::ZERO,
    );
    let hex = hex::encode(calldata.as_ref());

    assert!(hex.starts_with("414bf389"));
    assert_eq!(
        &hex[8..72],
        "0000000000000000000000000000000000000000000000000000000000000002"
    );
    assert_eq!(
        &hex[72..136],
        "0000000000000000000000000000000000000000000000000000000000000003"
    );
    assert_eq!(&hex[136..200], &word(2500));
    assert_eq!(
        &hex[200..264],
        "0000000000000000000000000000000000000000000000000000000000000004"
    );
    assert_eq!(&hex[264..328], &word(1_800_000_000));
    assert_eq!(&hex[328..392], &word(100));
    assert_eq!(&hex[392..456], &word(90));
    assert_eq!(&hex[456..520], &word(0));
}

#[test]
fn exact_input_calldata_encodes_dynamic_path_tail() {
    let path = bytes_from_hex(
        "0x00000000000000000000000000000000000000020009c40000000000000000000000000000000000000003",
        "encoded_path",
    )
    .expect("path should decode");
    let recipient = parse_address("0x0000000000000000000000000000000000000004", "recipient")
        .expect("recipient should parse");

    let calldata = encode_exact_input(
        &path,
        recipient,
        U256::from(1_800_000_000u64),
        U256::from(100),
        U256::from(90),
    );
    let hex = hex::encode(calldata.as_ref());

    assert!(hex.starts_with("c04b8d59"));
    assert_eq!(&hex[8..72], &word(160));
    assert_eq!(
        &hex[72..136],
        "0000000000000000000000000000000000000000000000000000000000000004"
    );
    assert_eq!(&hex[136..200], &word(1_800_000_000));
    assert_eq!(&hex[200..264], &word(100));
    assert_eq!(&hex[264..328], &word(90));
    assert_eq!(&hex[328..392], &word(path.len() as u64));
    assert_eq!(&hex[392..478], &hex::encode(path.as_ref()));
    assert_eq!(&hex[478..520], &"0".repeat(42));
    assert_eq!(hex.len(), 520);
}

fn word(value: u64) -> String {
    format!("{value:064x}")
}
