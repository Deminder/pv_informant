use crate::context::Context;
use crate::errors::ApiError;
use crate::influx_gateway::{query_pv_excess, ExcessStatus};
use crate::server::RequestHandler;
use crate::fwd_err;
use async_trait::async_trait;

pub struct ExcessRequestHandler {}

#[async_trait]
impl RequestHandler<String, ExcessStatus> for ExcessRequestHandler {
    async fn handle(&self, _query_str: String, context: Context) -> Result<ExcessStatus, ApiError> {
        Ok(query_pv_excess(&context.influx_client)
            .await
            .map_err(|e| fwd_err!("Failed to query pv excess! {}", e))?)
    }
}

#[cfg(test)]
mod test {
    // use super::*;
}
