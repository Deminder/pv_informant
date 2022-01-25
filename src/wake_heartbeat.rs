use crate::context::Context;
use crate::influx_gateway::query_wake_candidates;
use crate::influx_gateway::{log_workerstatus, query_pv_excess, WorkerStatus};
use crate::neighbor::{macs_to_addrs, awake_macs, wake_macs};
use futures::future::join_all;
use log::{error, info};
use std::collections::HashSet;

async fn waker_heartbeat(context: Context) {
    // ping macs that were not inquisitive for 10m
    let macs = query_wake_candidates(&context.influx_client)
        .await
        .unwrap_or_else(|e| {
            error!("Wake candidate query failed! {}", e);
            HashSet::new()
        });
    let woken_macs = match macs_to_addrs(&macs).await {
        Ok(mac_mapping) => {
            let macs_asleep = awake_macs(mac_mapping).await;
            for r in join_all(macs.iter().map(|mac| {
                log_workerstatus(
                    mac,
                    if macs_asleep.contains_key(mac) {
                        WorkerStatus::Sleep
                    } else {
                        WorkerStatus::Awake
                    },
                    true,
                    &context.influx_client,
                )
            }))
            .await
            {
                if let Err(e) = r {
                    error!("Failed logging workerstatus! {}", e)
                }
            }
            // wake asleep macs if excess = Yes
            match query_pv_excess(&context.influx_client).await {
                Ok(excess) => {
                    info!("PV excess: {}", excess as u8);
                    match wake_macs(&macs_asleep).await {
                        Ok(_) => macs_asleep.into_keys().collect(),
                        Err(e) => {
                            error!("Waking failed! {}", e);
                            HashSet::new()
                        }
                    }
                }
                Err(e) => {
                    error!("pv excess query failed! {}", e);
                    HashSet::new()
                }
            }
        }
        Err(e) => {
            error!("Exception while IP-addr lookup of wake candidates! {}", e);
            HashSet::new()
        }
    };
    context.just_woke(woken_macs)
}

pub async fn wake_heartbeat_loop(context: Context) -> Result<(), hyper::Error> {
    let mut interval = tokio::time::interval(context.wake_interval);
    while context.wake_interval_enabled {
        interval.tick().await;
        waker_heartbeat(context.clone()).await;
    }
    Ok(())
}
