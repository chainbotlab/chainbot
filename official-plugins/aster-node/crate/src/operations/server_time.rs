use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;
use crate::provider;

pub async fn handle(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    let output = provider::get_server_time(request.input_string("base_url")).await?;
    Ok(PluginResponse::success(output))
}
