# PV Informant
Informant for photo-voltaic data from influxdb.
Exposes simple JSON-API to query InfluxDB instance for photovoltaic (PV) data and register clients to Wake-On-LAN (WoL).

## Features

- Query PV power availability and register for WoL
  - Informant decides heuristically whether excess PV power is available
  - Woken over Ethernet via magic packet if PV power is available
- Report work activity by MAC
- Query time intervals of working status of a MAC
- Query time intervals of PV data

- Configure InfluxDB with: `INFLUXDB_CLIENT=user:password@http://host:port:dbname`
  - Use measurements `workerstatus` and `pvstatus`

## 
- mDNS support (with avahi) `pv-informant._http._tcp.local`
- TLS support (`CERT_FILE=/path/to/informant.cert.pem`)

### InfluxDB Schema
Used fields of influxDB pv measurement:
`pvstatus fields: [battery_voltage, pv_voltage, pv_current, temperature]`
`workerstatus tags: [mac] fields: [status]`

