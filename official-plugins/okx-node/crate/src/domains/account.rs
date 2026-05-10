use std::collections::BTreeMap;

use reqwest::Client;

use crate::contract::{PluginRequest, PluginResponse};
use crate::domains::{push_optional_query_param, request_context, require_api_key, require_api_secret, require_passphrase, success};
use crate::errors::PluginError;
use crate::provider::signed_get;

pub async fn get_account_balance(client: &Client, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
    let context = request_context(request)?;
    let api_key = require_api_key(request)?;
    let api_secret = require_api_secret(request)?;
    let passphrase = require_passphrase(request)?;
    let mut query = Vec::new();
    push_optional_query_param(request, "ccy", "ccy", &mut query);
    let payload = signed_get(
        client,
        &context,
        context.inst_type.account_balance_path(),
        query,
        api_key,
        api_secret,
        passphrase,
    )
    .await?;
    let balances = payload.get("data").cloned().unwrap_or(payload);
    Ok(success(BTreeMap::from([(String::from("balances"), balances)]), None))
}
