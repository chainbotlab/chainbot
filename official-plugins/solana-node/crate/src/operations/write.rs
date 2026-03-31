use base64::Engine;
use reqwest::Client;
use serde_json::Value;
use solana_instruction::Instruction;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::operations::{success, write_output};
use crate::provider::{
    fetch_latest_blockhash, parse_keypair_from_secret, parse_lamports, parse_pubkey, send_transaction,
    wait_for_signature_status,
};

pub async fn transfer_native(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let to = parse_pubkey(
        request
            .input_string("to")
            .ok_or_else(|| PluginError::InvalidInput(String::from("to is required")))?,
        "to",
    )?;
    let lamports = parse_lamports(
        request
            .input_string("lamports")
            .ok_or_else(|| PluginError::InvalidInput(String::from("lamports is required")))?,
        "lamports",
    )?;
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("confirmed");
    let preflight = request.input_bool("preflight").unwrap_or(true);
    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation.secrets.signer is required")))?;
    let signer = parse_keypair_from_secret(signer_secret)?;

    let instruction = system_instruction::transfer(&signer.pubkey(), &to, lamports);
    let tx = build_signed_transaction(client, endpoint, &signer, &[instruction]).await?;
    submit_transaction(client, endpoint, tx, confirmation_mode, preflight).await
}

pub async fn raw_write(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let endpoint = request
        .input_string("endpoint")
        .ok_or_else(|| PluginError::InvalidInput(String::from("endpoint is required")))?;
    let method = request
        .input_string("method")
        .ok_or_else(|| PluginError::InvalidInput(String::from("method is required")))?;
    if method != "sendTransaction" {
        return Err(PluginError::Unsupported(format!(
            "raw_write only supports sendTransaction in the managed signing path, got {method}"
        )));
    }

    let params = request
        .input
        .get("params")
        .and_then(Value::as_array)
        .ok_or_else(|| PluginError::InvalidInput(String::from("params must be an array")))?;
    let managed_payload = params.first().ok_or_else(|| {
        PluginError::InvalidInput(String::from("params[0] managed payload is required"))
    })?;
    let message_base64 = managed_payload
        .get("message_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PluginError::InvalidInput(String::from(
                "params[0].message_base64 is required for managed signing",
            ))
        })?;
    let message_bytes = base64::engine::general_purpose::STANDARD.decode(message_base64)?;
    let message: Message = bincode::deserialize(&message_bytes)?;

    let signer_secret = request
        .activation_secret("signer")
        .ok_or_else(|| PluginError::InvalidInput(String::from("activation.secrets.signer is required")))?;
    let signer = parse_keypair_from_secret(signer_secret)?;
    if message.account_keys.first() != Some(&signer.pubkey()) {
        return Err(PluginError::InvalidInput(String::from(
            "managed message fee payer must match activation signer",
        )));
    }

    let recent_blockhash = message.recent_blockhash.clone();
    let tx = Transaction::new(&[&signer], message, recent_blockhash);
    let confirmation_mode = request.input_string("confirmation_mode").unwrap_or("confirmed");
    let preflight = request.input_bool("preflight").unwrap_or(true);
    submit_transaction(client, endpoint, tx, confirmation_mode, preflight).await
}

async fn build_signed_transaction(
    client: &Client,
    endpoint: &str,
    signer: &solana_keypair::Keypair,
    instructions: &[Instruction],
) -> Result<Transaction, PluginError> {
    let recent_blockhash = fetch_latest_blockhash(client, endpoint).await?;
    Ok(Transaction::new_signed_with_payer(
        instructions,
        Some(&signer.pubkey()),
        &[signer],
        recent_blockhash,
    ))
}

async fn submit_transaction(
    client: &Client,
    endpoint: &str,
    transaction: Transaction,
    confirmation_mode: &str,
    preflight: bool,
) -> Result<PluginResponse, PluginError> {
    let signature = send_transaction(client, endpoint, &transaction, confirmation_mode, preflight).await?;
    if confirmation_mode == "submit_only" {
        return Ok(success(
            write_output("submitted", signature.clone(), confirmation_mode),
            Some("submitted"),
        ));
    }

    wait_for_signature_status(client, endpoint, &signature).await?;
    Ok(success(
        write_output("settled", signature, confirmation_mode),
        Some("settled"),
    ))
}

#[allow(dead_code)]
fn _assert_pubkey_is_english_only(_pubkey: &Pubkey) {}
