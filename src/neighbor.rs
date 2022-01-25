use anyhow::{Context, Result};
use mac_address::MacAddress;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::process::Stdio;
use tokio::net::UdpSocket;
use tokio::process::Command;
use wake_on_lan;

pub type MacIpMapping = HashMap<MacAddress, Option<IpAddr>>;

#[async_trait]
pub trait NetworkGateway {
    async fn ping(&self, ip: IpAddr) -> Result<bool, std::io::Error>;
    async fn ip_neigh(&self) -> Result<String>;
}

struct LinuxNetworkGateway {}

const LINUX_NET: &LinuxNetworkGateway = &LinuxNetworkGateway {};

#[async_trait]
impl NetworkGateway for LinuxNetworkGateway {
    async fn ping(&self, ip: IpAddr) -> Result<bool, std::io::Error> {
        Command::new("ping")
            .args(&[&ip.to_string(), "-c", "1", "-W", "1"])
            .stdout(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
    }
    async fn ip_neigh(&self) -> Result<String> {
        Ok(String::from_utf8(
            Command::new("ip")
                .arg("neigh")
                .output()
                .await
                .with_context(|| "/usr/sbin/ip failed")?
                .stdout,
        )?)
    }
}

pub async fn macs_to_addrs(macs: &HashSet<MacAddress>) -> Result<MacIpMapping> {
    _macs_to_addrs(macs, LINUX_NET).await
}

pub async fn addr_to_mac(addr: std::net::IpAddr) -> Result<Option<MacAddress>> {
    _addr_to_mac(addr, LINUX_NET).await
}

async fn _macs_to_addrs(
    macs: &HashSet<MacAddress>,
    net: &impl NetworkGateway,
) -> Result<MacIpMapping> {
    let mut addrs: HashMap<MacAddress, Option<IpAddr>> =
        macs.iter().map(|m| (m.clone(), None)).collect();
    for line in net.ip_neigh().await?.split("\n") {
        let mut segs = line.split(" ");
        let ip_addr = segs.next();
        let mut found = false;
        for s in segs {
            if found {
                let mac: MacAddress = s.parse()?;
                if macs.contains(&mac) {
                    addrs.insert(mac, ip_addr.unwrap().parse().ok());
                    break;
                }
            }
            found = s == "lladdr";
        }
    }
    Ok(addrs)
}

async fn _addr_to_mac(
    addr: std::net::IpAddr,
    net: &impl NetworkGateway,
) -> Result<Option<MacAddress>> {
    // no mac address lookup for loopback and multicast
    if addr.is_loopback() || addr.is_multicast() {
        return Ok(None);
    }
    for line in net.ip_neigh().await?.split("\n") {
        let mut segs = line.split(" ");
        if let Some(ip_addr_str) = segs.next() {
            if let Ok(ip_addr) = ip_addr_str.parse::<IpAddr>() {
                if ip_addr == addr {
                    let mut found = false;
                    for s in segs {
                        if found {
                            return Ok(s.parse().ok());
                        }
                        found = s == "lladdr";
                    }
                }
            }
        }
    }
    Ok(None)
}

pub async fn awake_macs(mac_mapping: MacIpMapping) -> HashMap<MacAddress, Option<IpAddr>> {
    _awake_macs(mac_mapping, LINUX_NET).await
}

async fn _awake_macs(
    mac_mapping: MacIpMapping,
    net: &impl NetworkGateway,
) -> HashMap<MacAddress, Option<IpAddr>> {
    // remove awake: macs which respond to ping are awake (ip-address from arp-table)
    let mut sleeping = HashMap::new();
    for (mac, ip_opt) in mac_mapping.into_iter() {
        sleeping.insert(
            mac,
            match ip_opt {
                // interpret mac/ip as sleeping if ping not successful
                Some(ip) if net.ping(ip).await.unwrap_or(false) => Some(ip),
                _ => None,
            },
        );
    }
    sleeping
}

fn addr_to_broadcast(ip_opt: &Option<IpAddr>) -> IpAddr {
    match ip_opt {
        Some(IpAddr::V4(ip)) => {
            let i: [u8; 4] = ip.octets();
            // assume subnet a.b.c.1/24 => broadcast a.b.c.255
            IpAddr::V4(Ipv4Addr::new(i[0], i[1], i[2], 255))
        }
        _ => IpAddr::V4(Ipv4Addr::BROADCAST),
    }
}

pub async fn wake_macs(mac_mapping: &HashMap<MacAddress, Option<IpAddr>>) -> Result<()> {
    // send magic packet to sleeping macs
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(10));
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_broadcast(true)?;
    for (m, ip_opt) in mac_mapping {
        let pkt = wake_on_lan::MagicPacket::new(&m.bytes());
        let brd_ip: IpAddr = addr_to_broadcast(ip_opt);
        interval.tick().await;
        socket
            .send_to(pkt.magic_bytes(), SocketAddr::new(brd_ip, 9))
            .await?;
        info!(
            "Waking {} with {} ({})",
            m,
            brd_ip,
            ip_opt
                .map(|i| i.to_string())
                .unwrap_or("ip not available".into())
        );
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[tokio::test]
    async fn test_net_commands() {
        let r = LINUX_NET.ip_neigh().await;
        assert!(r.is_ok());
        let r2 = LINUX_NET.ping(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
        assert!(r2.unwrap());
    }

    struct NetworkGatewayMock {
        ping_resp: HashMap<IpAddr, bool>,
        neigh_resp: String,
    }

    #[async_trait]
    impl NetworkGateway for NetworkGatewayMock {
        async fn ping(&self, ip: IpAddr) -> Result<bool, std::io::Error> {
            if ip.is_multicast() {
                println!("(mocked) BAD ping: {}", ip);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "mocking failed ping!",
                ))
            } else {
                println!("(mocked) ping: {}", ip);
                Ok(self.ping_resp[&ip])
            }
        }
        async fn ip_neigh(&self) -> Result<String> {
            Ok(self.neigh_resp.clone())
        }
    }
    macro_rules! neigh_resp {
        ( $value:literal ) => {
            &NetworkGatewayMock {
                ping_resp: HashMap::new(),
                neigh_resp: $value.into(),
            }
        };
    }

    #[tokio::test]
    async fn test_ip_to_mac() {
        let bad_sample = neigh_resp!(
            r#"
192.168.178.10 dev enp4s0 lladdr 12:34:56:78:9a:bb REACHABLE
192.168.178.1 dev enp4s0 lladdr 12:34:56:78:9a:xx REACHABLE
        "#
        );
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(_addr_to_mac(ip("192.168.178.1"), bad_sample).await.is_err());
        let sample = neigh_resp!(
            r#"
192.168.178.26 dev enp4s0 lladdr 12:34:56:78:9a:bc REACHABLE
192.168.178.1 dev enp4s0 lladdr 44:55:66:77:88:99 REACHABLE
fe80::abcd:abcd:abcd:abcd dev enp4s0 lladdr 44:4e:6d:c2:37:4b router DELAY
2a04:4540:4540:4540:4540:4540:4540:4540 dev enp4s0 lladdr 11:22:33:44:55:66 router REACHABLE
        "#
        );
        assert!(_addr_to_mac(ip("192.168.178.55"), sample)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            _addr_to_mac(ip("192.168.178.26"), sample)
                .await
                .unwrap()
                .unwrap()
                .to_string(),
            "12:34:56:78:9A:BC"
        );
        assert_eq!(
            _addr_to_mac(ip("2a04:4540:4540:4540:4540:4540:4540:4540"), sample)
                .await
                .unwrap()
                .unwrap()
                .to_string(),
            "11:22:33:44:55:66"
        );
    }
    #[tokio::test]
    async fn test_macs_to_ips() {
        let bad_sample = neigh_resp!(
            r#"
192.168.178.2 dev enp4s0 lladdr 11:11:11:11:11:11 REACHABLE
192.168.178.1 dev enp4s0 lladdr 12:34:xx:xx:9a:bc REACHABLE
        "#
        );
        let mac = |s: &str| s.parse::<MacAddress>().unwrap();
        let macs: HashSet<MacAddress> = [
            "11:11:11:11:11:11",
            "12:34:56:78:9a:bc",
            "44:55:66:77:88:99",
            "11:22:33:44:55:66",
        ]
        .into_iter()
        .map(mac)
        .collect();
        // invalid mac
        assert!(_macs_to_addrs(&macs, bad_sample).await.is_err());
        let sample = neigh_resp!(
            r#"
192.168.178.2 dev enp4s0 lladdr 22:22:22:22:22:22 REACHABLE
192.168.178.x dev enp4s0 lladdr 11:11:11:11:11:11 REACHABLE
192.168.178.26 dev enp4s0 lladdr 12:34:56:78:9a:bc REACHABLE
192.168.178.1 dev enp4s0 lladdr 44:55:66:77:88:99 REACHABLE
fe80::abcd:abcd:abcd:abcd dev enp4s0 lladdr 44:4e:6d:c2:37:4b router DELAY
2a04:4540:4540:4540:4540:4540:4540:4540 dev enp4s0 lladdr 11:22:33:44:55:66 router REACHABLE
        "#
        );
        let r = _macs_to_addrs(&macs, sample).await.unwrap();
        assert!(
            r.get(&mac("22:22:22:22:22:22")).is_none(),
            "should map non-searched ips to None"
        );
        assert!(
            r[&mac("11:11:11:11:11:11")].is_none(),
            "should map invalid ip to None"
        );
        // find searched

        for (m, expected_ip) in [
            ("12:34:56:78:9a:bc", "192.168.178.26"),
            ("44:55:66:77:88:99", "192.168.178.1"),
            (
                "11:22:33:44:55:66",
                "2a04:4540:4540:4540:4540:4540:4540:4540",
            ),
        ] {
            assert_matches!(
                r[&mac(m)],
                Some(ip) if ip.to_string() == expected_ip,
                "should map ip of searched mac to Some(expected_ip)"
            );
        }
    }
    #[tokio::test]
    async fn test_awake_macs() {
        macro_rules! ping_resp {
            ( $value:expr ) => {
                &NetworkGatewayMock {
                    ping_resp: $value.into_iter().collect(),
                    neigh_resp: "".into(),
                }
            };
        }
        let awake_ip: IpAddr = "192.168.178.22".parse().unwrap();
        let sleep_ip: IpAddr = "192.168.178.23".parse().unwrap();
        let sleep_ip2: IpAddr = "fe80::abcd:abcd:abcd:abcd".parse().unwrap();
        let failing_ip: IpAddr = "224.254.0.0".parse().unwrap();
        let net = ping_resp!([
            (awake_ip.clone(), true),
            (sleep_ip.clone(), false),
            (sleep_ip2.clone(), false),
            (failing_ip.clone(), true),
        ]);
        let awake_mac: MacAddress = "12:34:56:78:9a:bc".parse().unwrap();
        let none_mac_mapping: MacIpMapping = [(awake_mac.clone(), None)].into_iter().collect();
        assert_eq!(
            _awake_macs(none_mac_mapping.clone(), net).await,
            none_mac_mapping,
            "should leave None values in mapping"
        );
        let sleep_mac: MacAddress = "12:34:56:78:9a:bc".parse().unwrap();
        let sleep_mac2: MacAddress = "23:23:23:23:23:23".parse().unwrap();
        let failing_mac: MacAddress = "33:33:33:33:33:33".parse().unwrap();
        let mac_mapping: MacIpMapping = [
            (awake_mac.clone(), Some(awake_ip.clone())),
            (sleep_mac.clone(), Some(sleep_ip.clone())),
            (sleep_mac2.clone(), Some(sleep_ip2.clone())),
            ("22:22:22:22:22:22".parse().unwrap(), None),
            (failing_mac.clone(), Some(failing_ip.clone())),
        ]
        .into_iter()
        .collect();
        let mut expected_mapping = mac_mapping.clone();
        expected_mapping.insert(sleep_mac, None);
        expected_mapping.insert(sleep_mac2, None);
        expected_mapping.insert(failing_mac, None);

        assert_eq!(
            _awake_macs(mac_mapping, net).await,
            expected_mapping,
            "should set all ips of sleeping macs to None"
        );
    }

    #[test]
    fn test_addr_to_broadcast() {
        assert_eq!(addr_to_broadcast(&None).to_string(), "255.255.255.255");
        assert_eq!(addr_to_broadcast(&"fe80::abcd:abcd:abcd:abcd".parse().ok()).to_string(), "255.255.255.255");
        assert_eq!(addr_to_broadcast(&"192.168.178.23".parse().ok()).to_string(), "192.168.178.255");
        assert_eq!(addr_to_broadcast(&"192.168.122.55".parse().ok()).to_string(), "192.168.122.255");
    }
}
