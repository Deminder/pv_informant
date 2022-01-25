use crate::errors::ApiError;
use crate::neighbor::addr_to_mac;
use crate::server_err;
use mac_address::MacAddress;
use std::collections::HashSet;
use std::env;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct InfluxClient {
    pub client: influxdb::Client,
    pub workerstatus: String,
    pub pvstatus: String,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub influx_client: InfluxClient,
    pub wake_interval: std::time::Duration,
    pub wake_interval_enabled: bool,
    pub local_addr: std::net::SocketAddr,
    pub remote_addr: Option<std::net::SocketAddr>,
    // issued last wake in last heartbeat
    just_woke: Arc<Mutex<HashSet<MacAddress>>>,
}

impl Context {
    pub fn load() -> Result<Self, String> {
        Ok(Self {
            influx_client: InfluxClient {
                client: parse_influx_client(
                    env::var("INFLUXDB_CLIENT").unwrap_or("http://127.0.0.1:8086:test".into()),
                )?,
                workerstatus: env::var("WORKER_MEASUREMENT").unwrap_or("workerstatus".into()),
                pvstatus: env::var("PV_MEASUREMENT").unwrap_or("pvstatus".into()),
            },
            wake_interval: std::time::Duration::from_secs(
                env::var("WAKE_INTERVAL_SECONDS")
                    .unwrap_or("600".into())
                    .parse()
                    .map_err(|e| format!("Invalid wake interval seconds config! {}", e))?,
            ),
            wake_interval_enabled: env::var("DISABLE_WAKE_INTERVAL")
                .map(|_| false)
                .unwrap_or(true),
            local_addr: env::var("HOST")
                .unwrap_or("127.0.0.1:3000".into())
                .parse()
                .map_err(|e| format!("Invalid host config! {}", e))?,
            just_woke: Arc::new(Mutex::new(HashSet::new())),
            remote_addr: None,
        })
    }
    pub fn woken_in_previous_heartbeat(&self, mac: &MacAddress) -> bool {
        let woken_macs = self.just_woke.lock().unwrap();
        woken_macs.contains(mac)
    }
    pub fn just_woke(&self, macs: HashSet<MacAddress>) -> () {
        let mut guard = self.just_woke.lock().unwrap();
        *guard = macs;
    }
    pub async fn remote_mac(&self) -> Result<Option<MacAddress>, ApiError> {
        let ip = self.remote_addr.unwrap().ip();
        addr_to_mac(ip)
            .await
            .map_err(|e| server_err!("Failed to find mac for {}! {}", ip, e))
    }
}

fn parse_influx_client(influxdb_str: String) -> Result<influxdb::Client, String> {
    let error_str = "Invalid influxdb client config!";
    // user:password@http[s]://host:port:dbname
    // user:password and port is optional
    let mut auth_n_conn: Vec<&str> = influxdb_str.split('@').collect();
    let conn = auth_n_conn.pop().ok_or(error_str)?;
    let mut url_dbname: Vec<&str> = conn.split(':').collect();
    let dbname: &str = url_dbname.pop().ok_or(error_str)?;
    let url = url_dbname.join(":");
    let client = influxdb::Client::new(url, dbname);
    Ok(if auth_n_conn.len() > 0 {
        let auth = auth_n_conn[0];
        let mut name_pwd = auth.split(':');
        let username = name_pwd.next().ok_or(error_str)?;
        let password = name_pwd.next().unwrap_or(username);
        client.with_auth(username, password)
    } else {
        client
    })
}
#[cfg(test)]
mod test {}
