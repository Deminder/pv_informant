use crate::context::Context;
use crate::errors::ApiError;
use crate::influx_gateway::{log_workerstatus, WorkerStatus};
use crate::server::RequestHandler;
use crate::{api_err, fwd_err};
use async_trait::async_trait;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ReportRes {
    // has the mac of the requester recently been woken by magic packet
    woken: bool,
}

#[derive(Deserialize)]
pub struct ReportReq {
    status: WorkerStatus,
    wake: bool,
}

pub struct ReportRequestHandler {}

#[async_trait]
impl RequestHandler<ReportReq, ReportRes> for ReportRequestHandler {
    async fn handle(&self, req: ReportReq, context: Context) -> Result<ReportRes, ApiError> {
        // mac is required for report
        let mac = context.remote_mac().await?.ok_or_else(|| {
            api_err!(StatusCode::FORBIDDEN, "mac address of requestor not found!")
        })?;
        log_workerstatus(&mac, req.status, req.wake, &context.influx_client)
            .await
            .map_err(|e| fwd_err!("Failed to log reported status! {}", e))?;
        Ok(ReportRes {
            woken: context.woken_in_previous_heartbeat(&mac),
        })
    }
}

#[cfg(test)]
mod test {
    // use super::*;
}
