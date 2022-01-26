use crate::context::Context;
use crate::influx_gateway::{log_workerstatus, query_pv_excess, WorkerStatus};
use crate::influx_gateway::{query_stale_macs, ExcessStatus};
use crate::neighbor::{macs_to_addrs, sleeping_macs, wake_macs};
use log::{error, info};
use std::collections::HashSet;

async fn waker_heartbeat(context: Context) {
    // gather stale macs (not inquisitive for 10m) or already stale
    let stale_macs = query_stale_macs(&context.influx_client)
        .await
        .unwrap_or_else(|e| {
            error!("Stale macs query failed! {}", e);
            Vec::new()
        });
    let mut wake_candidates = HashSet::new();
    let mut logs = vec![];
    for (m, wake) in stale_macs {
        if wake {
            // ping macs with wake = true
            wake_candidates.insert(m);
        } else {
            // do not ping macs with wake = false 
            logs.push((m, WorkerStatus::Sleep, false));
        }
    }
    let mac_mapping = macs_to_addrs(&wake_candidates).await;
    let sleeping_macs = match &mac_mapping {
        Ok(mac_map) => sleeping_macs(mac_map).await,
        Err(e) => {
            error!("Exception while IP-addr lookup of wake candidates! {}", e);
            HashSet::new()
        }
    };

    // log new workerstatus
    for (m, s, w) in logs
        .into_iter()
        .chain(wake_candidates.into_iter().map(|mac| {
            (
                mac,
                if sleeping_macs.contains(&mac) {
                    WorkerStatus::Sleep
                } else {
                    WorkerStatus::Awake
                },
                true,
            )
        }))
    {
        if let Err(e) = log_workerstatus(&m, s, w, &context.influx_client).await {
            error!("Failed logging workerstatus! {}", e)
        }
    }

    let excess = match query_pv_excess(&context.influx_client).await {
        Ok(excess) => {
            info!("pv excess: {}", excess.clone() as u8);
            excess
        }
        Err(e) => {
            error!("pv excess query failed! {}", e);
            ExcessStatus::No
        }
    };

    // wake asleep macs if excess = Yes
    let woken_macs = match (excess, mac_mapping) {
        (ExcessStatus::Yes, Ok(mac_map)) => match wake_macs(&sleeping_macs, &mac_map).await {
            Ok(_) => sleeping_macs,
            Err(e) => {
                error!("Waking failed! {}", e);
                HashSet::new()
            }
        },
        _ => HashSet::new(),
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
