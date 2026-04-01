mod request;

use crate::contract::{PluginRequest, PluginResponse};
use crate::errors::PluginError;

pub async fn dispatch(request: PluginRequest) -> Result<PluginResponse, PluginError> {
    match request.operation.as_str() {
        "request" => request::handle(request).await,
        other => Err(PluginError::InvalidRequest(format!(
            "unsupported operation `{other}`"
        ))),
    }
}
