mod server_time;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    if request.contract_version != "1.0.0" {
        return Err(PluginError::InvalidRequest(format!(
            "unsupported contract_version {}",
            request.contract_version
        )));
    }
    if request.plugin_id != "aster-node" {
        return Err(PluginError::InvalidRequest(format!(
            "plugin_id must be `aster-node`, got `{}`",
            request.plugin_id
        )));
    }

    match request.operation.as_str() {
        "aster_get_server_time" => server_time::handle(request).await,
        other => Err(PluginError::InvalidRequest(format!(
            "unsupported operation `{other}`"
        ))),
    }
}
