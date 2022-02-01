# PV Informant
This informant may be queried if excess power of a photovoltaic (PV) installation is available.

To this end, it exposes a simple JSON-API to query an InfluxDB instance for PV data and periodically checks if PV excess is available to wake registered wokers with Wake-On-LAN (WoL).

## Features

- Query time intervals of influxdb measurements `pvstatus` and `workerstatus`
- Query availability of excess PV power (`Yes/Maybe/No`) 
  - Decided with thresholds of panel current and battery voltage from `pvstatus`
- Reported `work` (and `wake`) is logged to `workerstatus` 
  - Tagged with requestor MAC address
- Wakes clients/workers with `wake=true` over Ethernet via magic packet if PV excess available 

- Configure InfluxDB with: `INFLUXDB_CLIENT=user:password@http://host:port:dbname`

### InfluxDB Schema
Used influxdb measurement schema:
`pvstatus fields: [battery_voltage, pv_voltage, pv_current, temperature]`
`workerstatus tags: [mac] fields: [work, wake]`
