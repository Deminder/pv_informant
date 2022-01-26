use crate::context::InfluxClient;
use crate::interval_handler::IntervalReq;
use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use influxdb::{
    integrations::serde_integration::DatabaseQueryResult, InfluxDbWriteable, Query, ReadQuery,
};
use mac_address::MacAddress;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(InfluxDbWriteable)]
pub struct WorkerStatusEntry {
    #[influxdb(tag)]
    mac: String,
    time: DateTime<Utc>,
    status: i32,
    wake: bool,
}

#[async_trait]
pub trait QueryClient {
    async fn json_query(&self, query: ReadQuery) -> Result<DatabaseQueryResult, influxdb::Error>;
    async fn query<Q>(&self, query: Q) -> Result<String, influxdb::Error>
    where
        Q: Query + Send;
    fn workerstatus(&self) -> &str;
    fn pvstatus(&self) -> &str;
}

#[async_trait]
impl QueryClient for InfluxClient {
    async fn json_query(&self, query: ReadQuery) -> Result<DatabaseQueryResult, influxdb::Error> {
        self.client.json_query(query).await
    }
    async fn query<Q>(&self, q: Q) -> Result<String, influxdb::Error>
    where
        Q: Query + Send,
    {
        self.client.query(q).await
    }
    fn workerstatus(&self) -> &str {
        &self.workerstatus
    }
    fn pvstatus(&self) -> &str {
        &self.pvstatus
    }
}

#[derive(Debug, Deserialize, Clone)]
pub enum WorkerStatus {
    Sleep = 0,
    Awake = 1,
    Inquisitive = 2,
    Working = 3,
}

#[derive(Debug, Serialize, Clone)]
pub enum ExcessStatus {
    No = 0,
    Maybe = 1,
    Yes = 2,
}

// thresholds for battery_voltage depend on SUN_LEVEL based on pv_current
// 30m pv_current
const SUN_LEVELS: [f32; 3] = [7.0, 25.0, 40.0];
// 15m battery_voltage
const MAYBE_VOLTAGE_THRESHOLDS: [f32; 3] = [12.7, 12.5, 12.2];
const YES_VOLTAGE_THRESHOLDS: [f32; 3] = [13.2, 13.0, 12.7];

pub async fn query_pv_excess(c: &impl QueryClient) -> Result<ExcessStatus, influxdb::Error> {
    // query influxdb for excess pv power
    match mean_query(c, c.pvstatus(), "pv_current", "30m").await {
        Err(e) => Err(e),
        Ok(None) => {
            warn!("Could not determine mean of pv_current because of missing data!");
            Ok(ExcessStatus::No)
        }
        Ok(Some(mean_current)) => {
            let mut sun_level = 0;
            for (i, t) in IntoIterator::into_iter(SUN_LEVELS).enumerate() {
                if mean_current < t {
                    break;
                }
                sun_level = i + 1;
            }
            if sun_level < 1 {
                Ok(ExcessStatus::No)
            } else {
                match mean_query(c, c.pvstatus(), "battery_voltage", "15m").await {
                    Err(e) => Err(e),
                    Ok(None) => {
                        warn!(
                            "Could not determine mean of battery_voltage because of missing data!"
                        );
                        Ok(ExcessStatus::No)
                    }

                    Ok(Some(mean_voltage)) => {
                        Ok(if mean_voltage > YES_VOLTAGE_THRESHOLDS[sun_level - 1] {
                            ExcessStatus::Yes
                        } else if mean_voltage > MAYBE_VOLTAGE_THRESHOLDS[sun_level - 1] {
                            ExcessStatus::Maybe
                        } else {
                            ExcessStatus::No
                        })
                    }
                }
            }
        }
    }
}

pub async fn mean_query<Q>(
    c: &Q,
    measurement: &str,
    field: &str,
    duration: &str,
) -> Result<Option<f32>, influxdb::Error>
where
    Q: QueryClient,
{
    #[derive(Debug, Deserialize)]
    struct MeanMeasurement {
        mean: f32,
    }
    query_values::<MeanMeasurement, Q>(
        c,
        format!(
            "SELECT mean(\"{}\") AS mean FROM {} WHERE time > now() - {}",
            field, measurement, duration
        ),
    )
    .await
    .map(|values| values.into_iter().next())
    .map(|v| v.map(|m| m.mean))
}

pub async fn query_values<D: 'static, Q>(c: &Q, query: String) -> Result<Vec<D>, influxdb::Error>
where
    D: DeserializeOwned + Send,
    Q: QueryClient,
{
    c.json_query(ReadQuery::new(&query))
        .await
        .and_then(|mut db_result| db_result.deserialize_next::<D>())
        .map(|m| m.series.into_iter().next())
        .map(|s| match s {
            Some(s) => s.values,
            None => Vec::new(),
        })
}
pub async fn log_workerstatus(
    mac: &MacAddress,
    status: WorkerStatus,
    wake: bool,
    c: &impl QueryClient,
) -> Result<(), influxdb::Error> {
    // log workerstatus to influxdb
    let entry = WorkerStatusEntry {
        mac: mac.to_string(),
        time: DateTime::<Utc>::from(Local::now()),
        status: status as i32,
        wake,
    };
    info!("[{}] status: {}", mac, entry.status);
    c.query(entry.into_query(c.workerstatus())).await?;
    Ok(())
}

pub async fn query_history_interval(
    req: &IntervalReq,
    c: &impl QueryClient,
) -> Result<String, influxdb::Error> {
    let interval_query = req.query_condition();
    let query = ReadQuery::new(format!(
        "SELECT time, battery_voltage, pv_voltage, pv_current, temperature FROM {} WHERE {}",
        c.pvstatus(),
        interval_query
    ));
    c.query(if let Some(mac) = req.mac() {
        query.add_query(format!(
            "SELECT time, status, wake FROM {} WHERE {} AND mac = '{}'",
            c.workerstatus(),
            interval_query,
            mac
        ))
    } else {
        query
    })
    .await
}

pub async fn query_wake_candidates(
    c: &impl QueryClient,
) -> Result<HashSet<MacAddress>, influxdb::Error> {
    #[derive(Debug, Deserialize)]
    struct WorkerMac {
        mac: String,
    }
    let select_wake_macs = format!(
        // macs that want to be woken ( wake = true )
        "SELECT mac FROM (SELECT last(status) AS s,last(wake) AS w FROM {} GROUP BY mac) WHERE w = true AND",
        c.workerstatus()
    );
    let query = ReadQuery::new(format!(
        // stale working or inquisitive status
        "{} s >= {} AND time < now() - 10m",
        select_wake_macs,
        WorkerStatus::Inquisitive as u8,
    ))
    .add_query(format!(
        // non-inquisitive or non-working
        "{} s < {}",
        select_wake_macs,
        WorkerStatus::Inquisitive as u8,
    ));
    c.json_query(query)
        .await
        .and_then(|mut db_result| {
            let mut next_mac_iter = || db_result.deserialize_next::<WorkerMac>();
            Ok((next_mac_iter()?, next_mac_iter()?))
        })
        .map(|(r1, r2)| {
            r1.series
                .into_iter()
                .chain(r2.series.into_iter())
                .flat_map(|s| s.values.into_iter())
                .filter_map(|mac| mac.mac.parse().ok())
                .collect()
        })
}

#[cfg(test)]
pub mod test {

    use super::*;
    use async_trait::async_trait;
    use mac_address::MacAddress;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_log_workerstatus() {
        init_logger();
        let mac: MacAddress = "11:22:33:44:55:66".parse().unwrap();
        for status in [
            WorkerStatus::Sleep,
            WorkerStatus::Awake,
            WorkerStatus::Inquisitive,
            WorkerStatus::Working,
        ] {
            let write_query = format!(
                "workerstatus,mac={} status={}i,wake=true",
                mac,
                status.clone() as i32
            );
            info!(
                "Expecting '{}...' logging of {} for {}",
                write_query,
                status.clone() as i32,
                &mac
            );
            let client = InfluxClientMock {
                answer_map: HashMap::from([(write_query, "".into())]),
            };
            let r = log_workerstatus(&mac, status, true, &client).await;
            assert!(r.is_ok());
        }
    }

    #[tokio::test]
    async fn test_query_excess_pv() {
        const MEAN_RESP: &'static str = r#"[{
            "series": [{
                "name":"test_query",
                "columns": ["mean"],
                "values": [ 
                    [MEAN_VALUE]
                ]
            }]}]"#;

        macro_rules! mean_r {
            ($client:expr, $key:expr, $value:expr$(,)? ) => {
                $client.answer_map.insert(
                    $key.clone(),
                    MEAN_RESP.replace("MEAN_VALUE", &format!("{:.2}", $value + 0.01)),
                )
            };
        }
        init_logger();
        let pvcurrent_mean_query =
            "SELECT mean(\"pv_current\") AS mean FROM pvstatus WHERE time > now() - 30m"
                .to_string();
        let battery_voltage_mean_query =
            "SELECT mean(\"battery_voltage\") AS mean FROM pvstatus WHERE time > now() - 15m"
                .to_string();
        let mut client = InfluxClientMock {
            answer_map: HashMap::from([
                (
                    pvcurrent_mean_query.clone(),
                    "causes some influx error".into(),
                ),
                (
                    battery_voltage_mean_query.clone(),
                    "causes some influx error".into(),
                ),
            ]),
        };
        assert_matches!(
            query_pv_excess(&client).await,
            Err(_),
            "should not panic if queries fail"
        );

        mean_r!(client, pvcurrent_mean_query, 4.2);
        assert_matches!(
            query_pv_excess(&client).await.unwrap(),
            ExcessStatus::No,
            "should not call failing second query if the SUN_LEVEL indicates NIGHT"
        );
        mean_r!(client, pvcurrent_mean_query, SUN_LEVELS[0]);
        assert_matches!(
            query_pv_excess(&client).await,
            Err(_),
            "should call second (failing) query to check for YES/MAYBE excess"
        );
        mean_r!(
            client,
            battery_voltage_mean_query,
            MAYBE_VOLTAGE_THRESHOLDS[1]
        );
        assert_matches!(
            query_pv_excess(&client).await.unwrap(),
            ExcessStatus::No,
            "should have too low voltage for MAYBE with SUN_LEVEL[0]"
        );
        mean_r!(client, pvcurrent_mean_query, SUN_LEVELS[1]);
        assert_matches!(
            query_pv_excess(&client).await.unwrap(),
            ExcessStatus::Maybe,
            "should have enough voltage for MAYBE with SUN_LEVEL[1]"
        );
        mean_r!(
            client,
            battery_voltage_mean_query,
            YES_VOLTAGE_THRESHOLDS[1]
        );
        assert_matches!(
            query_pv_excess(&client).await.unwrap(),
            ExcessStatus::Yes,
            "should have enough voltage for YES with SUN_LEVEL[1]"
        );
    }

    #[tokio::test]
    async fn test_query_history_interval() {
        use chrono::{DateTime, Duration, Local, Utc};
        init_logger();
        let n = DateTime::<Utc>::from(Local::now());
        let req = IntervalReq::new(None, n, n + Duration::days(7));
        let query_output = "some query output";
        let client = InfluxClientMock {
            answer_map: HashMap::from([
                (
                    format!("SELECT time, battery_voltage, pv_voltage, pv_current, temperature FROM pvstatus WHERE {}", req.query_condition()),
                    query_output.into(),
                ),
            ]),
        };
        assert_matches!(
            query_history_interval(&req, &client).await,
            Ok(output) if output == query_output,
            "should query without workerstatus if mac is None"
        );
        let reqwithmac =
            IntervalReq::new("11:11:11:11:11:11".parse().ok(), n, n + Duration::days(7));
        let client_mac = InfluxClientMock {
            answer_map: HashMap::from([
                (
                    format!("SELECT time, battery_voltage, pv_voltage, pv_current, temperature FROM pvstatus WHERE {};SELECT time, status, wake FROM workerstatus WHERE {} AND mac = '{}'",
                        req.query_condition(), req.query_condition(), reqwithmac.mac().unwrap() ),
                    query_output.into(),
                ),
            ]),
        };
        assert_matches!(
            query_history_interval(&reqwithmac, &client_mac).await,
            Ok(output) if output == query_output,
            "should query with workerstatus if Some(mac)"
        );
    }

    #[tokio::test]
    async fn test_query_wake_candidates() {
        init_logger();
        for (a, b) in [
            (WorkerStatus::Sleep, WorkerStatus::Awake),
            (WorkerStatus::Awake, WorkerStatus::Inquisitive),
            (WorkerStatus::Inquisitive, WorkerStatus::Working),
        ] {
            assert!(
                { a as i32 } < { b as i32 },
                "should be able to order workerstatus correctly"
            );
        }

        let base_query = "SELECT mac FROM (SELECT last(status) AS s,last(wake) AS w FROM workerstatus GROUP BY mac) WHERE w = true AND";
        let inq = WorkerStatus::Inquisitive as i32;
        let query_output = r#"[{
            "series": [{
                "name":"workers_stale",
                "columns": ["mac"],
                "values": [ 
                ["11:22:33:44:55:66"], ["11:22:33:44:55:77"], ["11:22:33:44:55:88"]
                ]
            }]},{
            "series": [{
                "name":"workers_sleeping",
                "columns": ["mac"],
                "values": [ 
                ["11:22:33:44:55:99"], ["11:22:33:44:55:aa"], ["11:22:33:44:55:bb"]
                ]
            }]}]"#;
        let expected_candiates: HashSet<MacAddress> = [
            "11:22:33:44:55:66",
            "11:22:33:44:55:77",
            "11:22:33:44:55:88",
            "11:22:33:44:55:99",
            "11:22:33:44:55:aa",
            "11:22:33:44:55:bb",
        ]
        .into_iter()
        .map(|s| s.parse().unwrap())
        .collect();
        let client = InfluxClientMock {
            answer_map: HashMap::from([(
                format!(
                    "{} s >= {} AND time < now() - 10m;{} s < {}",
                    base_query, inq, base_query, inq
                ),
                query_output.into(),
            )]),
        };
        let candidates = query_wake_candidates(&client).await.unwrap();
        assert_eq!(
            candidates, expected_candiates,
            "should output all macs in response"
        );
    }

    pub struct InfluxClientMock {
        answer_map: HashMap<String, String>,
    }

    impl InfluxClientMock {
        pub fn query_result<Q>(&self, query: Q) -> Result<String, influxdb::Error>
        where
            Q: Query + Send,
        {
            let query_str = query.build()?.get();
            let key = if let influxdb::QueryType::ReadQuery = query.get_type() {
                if self.answer_map.contains_key(&query_str) {
                    Some(&query_str)
                } else {
                    None
                }
            } else {
                self.answer_map.keys().find(|k| query_str.starts_with(*k))
            };
            assert!(key.is_some(), "Incorrect query: '{}'", &query_str);
            let k = key.unwrap();
            debug!("Mock query resonse for: {}", k);
            Ok(self.answer_map[k].clone())
        }
    }
    fn init_logger() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[async_trait]
    impl QueryClient for InfluxClientMock {
        async fn json_query(
            &self,
            query: ReadQuery,
        ) -> Result<DatabaseQueryResult, influxdb::Error> {
            let res = self.query_result(query)?;
            let values: Vec<serde_json::Value> =
                serde_json::from_str(&res).map_err(|e| influxdb::Error::DeserializationError {
                    error: format!("Failed to deserialize '{}' (MOCKED)! {}", res, e),
                })?;
            Ok(DatabaseQueryResult { results: values })
        }
        async fn query<Q>(&self, q: Q) -> Result<String, influxdb::Error>
        where
            Q: Query + Send,
        {
            Ok(self.query_result(q)?.to_string())
        }
        fn workerstatus(&self) -> &str {
            "workerstatus"
        }
        fn pvstatus(&self) -> &str {
            "pvstatus"
        }
    }
}
