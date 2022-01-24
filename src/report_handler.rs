use crate::context::Context;
use crate::errors::ApiError;
use crate::influx_gateway::{log_workerstatus, query_pv_excess, ExcessStatus, WorkerStatus};
use crate::server::RequestHandler;
use crate::{api_err, fwd_err};
use async_trait::async_trait;
use futures::try_join;
use futures::TryFutureExt;
use hyper::StatusCode;
use mac_address::MacAddress;
use serde::Serialize;

#[derive(Serialize)]
pub struct ReportRes {
    excess: ExcessStatus,
    // has the mac of the requester recently been woken by magic packet
    woken: bool,
}

pub struct ReportRequestHandler {}

#[derive(Debug)]
enum ReportType {
    ExcessAndWake(MacAddress, WorkerStatus),
    Excess,
}

fn request_type(req: WorkerStatus, remote_mac: Option<MacAddress>) -> Result<ReportType, ApiError> {
    match (&req, remote_mac) {
        // if reporting nowake the mac is allowed to be missing
        (WorkerStatus::NoWake, None) | (_, Some(_)) => Ok(match remote_mac {
            Some(mac) => ReportType::ExcessAndWake(mac, req),
            None => ReportType::Excess,
        }),
        _ => Err(api_err!(
            StatusCode::FORBIDDEN,
            "mac address of requestor not found!"
        )),
    }
}

#[async_trait]
impl RequestHandler<WorkerStatus, ReportRes> for ReportRequestHandler {
    async fn handle(&self, req: WorkerStatus, context: Context) -> Result<ReportRes, ApiError> {
        match request_type(req, context.remote_mac().await?) {
            Ok(rtype) => {
                let query_excess = query_pv_excess(&context.influx_client)
                    .map_err(|e| fwd_err!("Failed to query pv excess! {}", e));
                let woken = match rtype {
                    ReportType::Excess => false,
                    ReportType::ExcessAndWake(m, _) => context.woken_in_previous_heartbeat(&m),
                };
                let excess = match rtype {
                    ReportType::Excess => query_excess.await?,
                    ReportType::ExcessAndWake(m, s) => {
                        try_join!(
                            log_workerstatus(&m, s, &context.influx_client)
                                .map_err(|e| fwd_err!("Failed to log reported status! {}", e)),
                            query_excess
                        )?
                        .1
                    }
                };
                Ok(ReportRes { woken, excess })
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mac_address::MacAddress;

    #[test]
    fn test_validate_request() {
        assert_matches!(
            request_type(WorkerStatus::NoWake, None).unwrap(),
            ReportType::Excess,
            "should process nowake request even if mac unavailable"
        );

        for req in [
            WorkerStatus::Sleep,
            WorkerStatus::Awake,
            WorkerStatus::Inquisitive,
        ] {
            assert_matches!(
                request_type(req, None),
                Err(_),
                "should not process non-nowake request if mac unavailable"
            );
        }

        let mac: MacAddress = "11:11:11:11:11:11".parse().ok().unwrap();

        for req in [
            WorkerStatus::NoWake,
            WorkerStatus::Sleep,
            WorkerStatus::Awake,
            WorkerStatus::Inquisitive,
        ] {
            assert_matches!(
                request_type(req.clone(), Some(mac.clone())).unwrap(),
                ReportType::ExcessAndWake(m, s) if (s.clone() as u8) == (req as u8) && m == mac,
                "should answer excess and wake if mac available"
            );
        }
    }
}
