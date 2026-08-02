use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use ferrobox_core::{NetworkMode, RuntimeError, RuntimeErrorKind, SandboxId};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream, UdpSocket},
    process::Command,
    sync::Semaphore,
    task::{AbortHandle, JoinSet},
    time::timeout,
};

const DNS_PORT: u16 = 53;
const DNS_MAX_MESSAGE_BYTES: usize = u16::MAX as usize;
const DNS_MAX_IN_FLIGHT: usize = 128;
const DNS_UPSTREAM_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct NetworkLease {
    pub namespace: String,
    pub namespace_path: PathBuf,
    pub tap_name: String,
    pub guest_mac: String,
    pub guest_address: String,
    pub gateway: String,
    pub dns_ipv4: String,
    host_veth: String,
    nft_table: String,
    #[cfg(target_os = "linux")]
    dns_relay: Option<DnsRelay>,
}

#[derive(Clone, Debug, Default)]
pub struct NetworkManager;

impl NetworkManager {
    pub async fn create(
        &self,
        sandbox_id: &SandboxId,
        mode: NetworkMode,
    ) -> Result<Option<NetworkLease>, RuntimeError> {
        if mode == NetworkMode::Disabled {
            return Ok(None);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = sandbox_id;
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unsupported,
                "network namespaces require Linux",
            ));
        }
        #[cfg(target_os = "linux")]
        self.create_linux(sandbox_id).await.map(Some)
    }

    #[cfg(target_os = "linux")]
    async fn create_linux(&self, sandbox_id: &SandboxId) -> Result<NetworkLease, RuntimeError> {
        let compact = sandbox_id.to_string().replace('-', "");
        let short = &compact[..8];
        let octet = (32 + (u16::from_str_radix(&compact[..2], 16).unwrap_or(0) % 191)) as u8;
        let namespace = format!("fb-{short}");
        let host_veth = format!("fbh{short}");
        let peer_veth = format!("fbn{short}");
        let nft_table = format!("ferrobox_{short}");
        let gateway_ipv4 = Ipv4Addr::new(10, 200, octet, 1);
        let guest_ipv4 = Ipv4Addr::new(10, 200, octet, 2);
        let gateway = gateway_ipv4.to_string();
        let guest_address = guest_ipv4.to_string();
        let subnet = format!("10.200.{octet}.0/24");
        let mut lease = NetworkLease {
            namespace_path: PathBuf::from(format!("/run/netns/{namespace}")),
            namespace: namespace.clone(),
            tap_name: "tap0".to_owned(),
            guest_mac: "06:00:ac:10:00:02".to_owned(),
            guest_address,
            gateway: gateway.clone(),
            dns_ipv4: gateway.clone(),
            host_veth: host_veth.clone(),
            nft_table: nft_table.clone(),
            dns_relay: None,
        };

        let commands: Vec<Vec<String>> = vec![
            vec!["ip".into(), "netns".into(), "add".into(), namespace.clone()],
            vec![
                "ip".into(),
                "link".into(),
                "add".into(),
                host_veth.clone(),
                "type".into(),
                "veth".into(),
                "peer".into(),
                "name".into(),
                peer_veth.clone(),
            ],
            vec![
                "ip".into(),
                "link".into(),
                "set".into(),
                peer_veth.clone(),
                "netns".into(),
                namespace.clone(),
            ],
            vec![
                "ip".into(),
                "address".into(),
                "add".into(),
                format!("{gateway}/24"),
                "dev".into(),
                host_veth.clone(),
            ],
            vec![
                "ip".into(),
                "link".into(),
                "set".into(),
                host_veth.clone(),
                "up".into(),
            ],
            netns(&namespace, &["ip", "link", "add", "br0", "type", "bridge"]),
            netns(
                &namespace,
                &["ip", "tuntap", "add", "dev", "tap0", "mode", "tap"],
            ),
            netns(&namespace, &["ip", "link", "set", "tap0", "master", "br0"]),
            netns(
                &namespace,
                &["ip", "link", "set", &peer_veth, "master", "br0"],
            ),
            netns(&namespace, &["ip", "link", "set", "lo", "up"]),
            netns(&namespace, &["ip", "link", "set", "br0", "up"]),
            netns(&namespace, &["ip", "link", "set", "tap0", "up"]),
            netns(&namespace, &["ip", "link", "set", &peer_veth, "up"]),
        ];
        for command in commands {
            if let Err(error) = run(&command).await {
                let _ = self.delete(&lease).await;
                return Err(error);
            }
        }

        let rules = format!(
            "table inet {nft_table} {{\n\
             chain input {{ type filter hook input priority -10; policy accept;\n\
             iifname \"{host_veth}\" ip daddr {gateway} udp dport {DNS_PORT} accept\n\
             iifname \"{host_veth}\" ip daddr {gateway} tcp dport {DNS_PORT} accept\n\
             iifname \"{host_veth}\" reject\n\
             }}\n\
             chain forward {{ type filter hook forward priority -10; policy accept;\n\
             iifname \"{host_veth}\" ip daddr {{ 0.0.0.0/8, 10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.0.0.0/24, 192.168.0.0/16, 224.0.0.0/4, 240.0.0.0/4 }} reject\n\
             iifname \"{host_veth}\" accept\n\
             oifname \"{host_veth}\" ct state established,related accept\n\
             }}\n\
             chain postrouting {{ type nat hook postrouting priority srcnat; policy accept;\n\
             ip saddr {subnet} oifname != \"{host_veth}\" masquerade\n\
             }}\n\
             }}\n"
        );
        if let Err(error) = run_nft(&rules).await {
            let _ = self.delete(&lease).await;
            return Err(error);
        }
        let relay_address = SocketAddr::from((gateway_ipv4, DNS_PORT));
        let upstreams = host_dns_upstreams().await;
        match DnsRelay::start(relay_address, guest_ipv4, upstreams).await {
            Ok(relay) => lease.dns_relay = Some(relay),
            Err(error) => {
                let _ = self.delete(&lease).await;
                return Err(error);
            }
        }
        Ok(lease)
    }

    pub async fn delete(&self, lease: &NetworkLease) -> Result<(), RuntimeError> {
        #[cfg(target_os = "linux")]
        {
            if let Some(relay) = &lease.dns_relay {
                relay.abort();
            }
            let _ = run(&[
                "nft".into(),
                "delete".into(),
                "table".into(),
                "inet".into(),
                lease.nft_table.clone(),
            ])
            .await;
            let _ = run(&[
                "ip".into(),
                "link".into(),
                "delete".into(),
                lease.host_veth.clone(),
            ])
            .await;
            let _ = run(&[
                "ip".into(),
                "netns".into(),
                "delete".into(),
                lease.namespace.clone(),
            ])
            .await;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn host_dns_upstreams() -> Vec<SocketAddr> {
    let mut upstreams = Vec::new();
    for path in ["/etc/resolv.conf", "/run/systemd/resolve/resolv.conf"] {
        if let Ok(contents) = tokio::fs::read_to_string(path).await {
            for address in ipv4_nameservers(&contents) {
                let upstream = SocketAddr::from((address, DNS_PORT));
                if !upstreams.contains(&upstream) {
                    upstreams.push(upstream);
                }
            }
        }
    }
    let fallback = SocketAddr::from((Ipv4Addr::new(1, 1, 1, 1), DNS_PORT));
    if !upstreams.contains(&fallback) {
        upstreams.push(fallback);
    }
    upstreams.truncate(3);
    upstreams
}

#[cfg(target_os = "linux")]
fn ipv4_nameservers(contents: &str) -> Vec<Ipv4Addr> {
    contents
        .lines()
        .filter(|line| line.split_whitespace().next() == Some("nameserver"))
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|value| value.parse::<std::net::Ipv4Addr>().ok())
        .filter(|address| {
            !address.is_unspecified() && !address.is_multicast() && *address != Ipv4Addr::BROADCAST
        })
        .collect()
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct DnsRelay {
    abort_handle: AbortHandle,
}

#[cfg(target_os = "linux")]
impl DnsRelay {
    async fn start(
        bind_address: SocketAddr,
        guest_ipv4: Ipv4Addr,
        upstreams: Vec<SocketAddr>,
    ) -> Result<Self, RuntimeError> {
        let udp = Arc::new(UdpSocket::bind(bind_address).await.map_err(|error| {
            RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                format!("bind sandbox DNS UDP relay: {error}"),
            )
        })?);
        let listen_address = udp.local_addr().map_err(|error| {
            RuntimeError::internal(format!("read sandbox DNS relay address: {error}"))
        })?;
        let tcp = TcpListener::bind(listen_address).await.map_err(|error| {
            RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                format!("bind sandbox DNS TCP relay: {error}"),
            )
        })?;
        let upstreams: Arc<[SocketAddr]> = upstreams.into();
        let permits = Arc::new(Semaphore::new(DNS_MAX_IN_FLIGHT));
        let task = tokio::spawn(async move {
            let mut udp_buffer = vec![0_u8; DNS_MAX_MESSAGE_BYTES];
            let mut queries = JoinSet::new();
            loop {
                tokio::select! {
                    received = udp.recv_from(&mut udp_buffer) => {
                        let Ok((length, peer)) = received else {
                            break;
                        };
                        if peer.ip() != IpAddr::V4(guest_ipv4)
                            || !is_dns_query(&udp_buffer[..length])
                        {
                            continue;
                        }
                        let Ok(permit) = permits.clone().try_acquire_owned() else {
                            continue;
                        };
                        let query = udp_buffer[..length].to_vec();
                        let udp = udp.clone();
                        let upstreams = upstreams.clone();
                        queries.spawn(async move {
                            let _permit = permit;
                            relay_udp_query(udp, peer, query, upstreams).await;
                        });
                    }
                    accepted = tcp.accept() => {
                        let Ok((stream, peer)) = accepted else {
                            break;
                        };
                        if peer.ip() != IpAddr::V4(guest_ipv4) {
                            continue;
                        }
                        let Ok(permit) = permits.clone().try_acquire_owned() else {
                            continue;
                        };
                        let upstreams = upstreams.clone();
                        queries.spawn(async move {
                            let _permit = permit;
                            relay_tcp_queries(stream, upstreams).await;
                        });
                    }
                    completed = queries.join_next(), if !queries.is_empty() => {
                        let _ = completed;
                    }
                }
            }
        });
        Ok(Self {
            abort_handle: task.abort_handle(),
        })
    }

    fn abort(&self) {
        self.abort_handle.abort();
    }
}

#[cfg(target_os = "linux")]
impl Drop for DnsRelay {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(target_os = "linux")]
async fn relay_udp_query(
    listener: Arc<UdpSocket>,
    peer: SocketAddr,
    query: Vec<u8>,
    upstreams: Arc<[SocketAddr]>,
) {
    for upstream in upstreams.iter().copied() {
        let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
            continue;
        };
        if socket.connect(upstream).await.is_err() || socket.send(&query).await.is_err() {
            continue;
        }
        let mut response = vec![0_u8; DNS_MAX_MESSAGE_BYTES];
        let received = timeout(DNS_UPSTREAM_TIMEOUT, socket.recv(&mut response)).await;
        let Ok(Ok(length)) = received else {
            continue;
        };
        if is_dns_response_for(&query, &response[..length]) {
            let _ = listener.send_to(&response[..length], peer).await;
            return;
        }
    }
    let _ = listener.send_to(&dns_servfail(&query), peer).await;
}

#[cfg(target_os = "linux")]
async fn relay_tcp_queries(mut client: TcpStream, upstreams: Arc<[SocketAddr]>) {
    loop {
        let mut length_bytes = [0_u8; 2];
        let read_length = timeout(DNS_UPSTREAM_TIMEOUT, client.read_exact(&mut length_bytes)).await;
        let Ok(Ok(_)) = read_length else {
            return;
        };
        let length = usize::from(u16::from_be_bytes(length_bytes));
        let mut query = vec![0_u8; length];
        let read_query = timeout(DNS_UPSTREAM_TIMEOUT, client.read_exact(&mut query)).await;
        if !matches!(read_query, Ok(Ok(_))) || !is_dns_query(&query) {
            return;
        }

        let response = relay_tcp_query(&query, &upstreams)
            .await
            .unwrap_or_else(|| dns_servfail(&query).to_vec());
        let Ok(response_length) = u16::try_from(response.len()) else {
            return;
        };
        if client
            .write_all(&response_length.to_be_bytes())
            .await
            .is_err()
            || client.write_all(&response).await.is_err()
        {
            return;
        }
    }
}

#[cfg(target_os = "linux")]
async fn relay_tcp_query(query: &[u8], upstreams: &[SocketAddr]) -> Option<Vec<u8>> {
    for upstream in upstreams.iter().copied() {
        let connected = timeout(DNS_UPSTREAM_TIMEOUT, TcpStream::connect(upstream)).await;
        let Ok(Ok(mut stream)) = connected else {
            continue;
        };
        let Ok(query_length) = u16::try_from(query.len()) else {
            return None;
        };
        let exchange = async {
            stream.write_all(&query_length.to_be_bytes()).await?;
            stream.write_all(query).await?;
            let mut length_bytes = [0_u8; 2];
            stream.read_exact(&mut length_bytes).await?;
            let response_length = usize::from(u16::from_be_bytes(length_bytes));
            let mut response = vec![0_u8; response_length];
            stream.read_exact(&mut response).await?;
            Ok::<_, std::io::Error>(response)
        };
        if let Ok(Ok(response)) = timeout(DNS_UPSTREAM_TIMEOUT, exchange).await
            && is_dns_response_for(query, &response)
        {
            return Some(response);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn is_dns_query(message: &[u8]) -> bool {
    message.len() >= 12
        && message[2] & 0x80 == 0
        && u16::from_be_bytes([message[4], message[5]]) > 0
}

#[cfg(target_os = "linux")]
fn is_dns_response_for(query: &[u8], response: &[u8]) -> bool {
    query.len() >= 2
        && response.len() >= 12
        && response[..2] == query[..2]
        && response[2] & 0x80 != 0
}

#[cfg(target_os = "linux")]
fn dns_servfail(query: &[u8]) -> [u8; 12] {
    let mut response = [0_u8; 12];
    if query.len() >= 2 {
        response[..2].copy_from_slice(&query[..2]);
    }
    response[2] = 0x80 | query.get(2).copied().unwrap_or_default() & 0x01;
    response[3] = 0x82;
    response
}

#[cfg(target_os = "linux")]
fn netns(namespace: &str, command: &[&str]) -> Vec<String> {
    let mut output = vec![
        "ip".to_owned(),
        "netns".to_owned(),
        "exec".to_owned(),
        namespace.to_owned(),
    ];
    output.extend(command.iter().map(|argument| (*argument).to_owned()));
    output
}

#[cfg(target_os = "linux")]
async fn run(command: &[String]) -> Result<(), RuntimeError> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| RuntimeError::internal("empty network command"))?;
    let output = Command::new(program)
        .args(arguments)
        .output()
        .await
        .map_err(|error| RuntimeError::internal(format!("run {program}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(RuntimeError::new(
            RuntimeErrorKind::Unavailable,
            format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

#[cfg(target_os = "linux")]
async fn run_nft(rules: &str) -> Result<(), RuntimeError> {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| RuntimeError::internal(format!("start nft: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| RuntimeError::internal("nft stdin unavailable"))?
        .write_all(rules.as_bytes())
        .await
        .map_err(|error| RuntimeError::internal(format!("write nft rules: {error}")))?;
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| RuntimeError::internal(format!("wait for nft: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(RuntimeError::new(
            RuntimeErrorKind::Unavailable,
            format!("nft failed: {}", String::from_utf8_lossy(&output.stderr)),
        ))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{dns_servfail, ipv4_nameservers, is_dns_query, is_dns_response_for};

    #[test]
    fn keeps_host_scoped_resolvers_behind_relay() {
        let input = "nameserver 127.0.0.53\nnameserver 10.0.0.2\nnameserver 168.63.129.16\n";
        assert_eq!(
            ipv4_nameservers(input),
            vec![
                "127.0.0.53".parse().expect("loopback resolver"),
                "10.0.0.2".parse().expect("private resolver"),
                "168.63.129.16".parse().expect("cloud resolver"),
            ]
        );
    }

    #[test]
    fn validates_query_response_pair_and_servfail() {
        let query = [0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        let mut response = query;
        response[2] |= 0x80;
        assert!(is_dns_query(&query));
        assert!(is_dns_response_for(&query, &response));
        assert_eq!(dns_servfail(&query)[..2], query[..2]);
        assert_eq!(dns_servfail(&query)[3] & 0x0f, 2);
    }
}
