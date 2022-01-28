use crate::context::Context;
use crate::errors::ApiError;
use crate::influx_gateway::query_history_interval;
use crate::server::RequestHandler;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use mac_address::MacAddress;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct IntervalReq {
    mac: Option<MacAddress>,
    start: DateTime<Utc>,
    stop: DateTime<Utc>,
}

impl IntervalReq {
    pub fn query_condition(&self) -> String {
        format!(
            "time > '{}' AND time < '{}'",
            self.start.to_rfc3339(),
            self.stop.to_rfc3339()
        )
    }
    pub fn mac(&self) -> Option<MacAddress> {
        self.mac
    }
}

const MAX_QUERY_DAYS: i64 = 20;
fn validate_request(req: &IntervalReq) -> Result<(), ApiError> {
    let dur = req.stop - req.start;
    if dur > Duration::days(MAX_QUERY_DAYS) {
        Err(api_baderr!("'{}' exceeded max query duration!", dur))
    } else {
        Ok(())
    }
}

pub struct IntervalRequestHandler {}

#[async_trait]
impl RequestHandler<IntervalReq, String> for IntervalRequestHandler {
    async fn handle(&self, req: IntervalReq, context: Context) -> Result<String, ApiError> {
        let mut req = req;
        if req.mac.is_none() {
            // try using the mac of the requester for query
            req.mac = context.remote_mac().await?;
        }
        if let Err(e) = validate_request(&req) {
            Err(e)
        } else {
            Ok(query_history_interval(&req, &context.influx_client)
                .await
                .map_err(|e| fwd_err!("Query failed! {}", e))?)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use chrono::{DateTime, Duration, Utc};

    impl IntervalReq {
        pub fn new(mac: Option<MacAddress>, start: DateTime<Utc>, stop: DateTime<Utc>) -> Self {
            IntervalReq { mac, start, stop }
        }
    }
    #[test]
    fn test_validation() {
        let n = Utc::now();
        let mut req = IntervalReq {
            mac: None,
            start: n,
            stop: n + Duration::days(MAX_QUERY_DAYS),
        };
        assert_matches!(validate_request(&req), Ok(()));
        req.stop = n + Duration::days(MAX_QUERY_DAYS + 1);
        assert_matches!(validate_request(&req), Err(_));
    }
}
