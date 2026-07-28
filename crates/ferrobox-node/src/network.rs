use std::{path::PathBuf, process::Stdio};

use ferrobox_core::{NetworkMode, RuntimeError, RuntimeErrorKind, SandboxId};
use tokio::{io::AsyncWriteExt as _, process::Command};

#[derive(Clone, Debug)]
pub struct NetworkLease {
    pub namespace: String,
    pub namespace_path: PathBuf,
    pub tap_name: String,
    pub guest_mac: String,
    pub guest_address: String,
    pub gateway: String,
    host_veth: String,
    nft_table: String,
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
        let octet = 32 + (u16::from_str_radix(&compact[..2], 16).unwrap_or(0) % 191);
        let namespace = format!("fb-{short}");
        let host_veth = format!("fbh{short}");
        let peer_veth = format!("fbn{short}");
        let nft_table = format!("ferrobox_{short}");
        let gateway = format!("10.200.{octet}.1");
        let guest_address = format!("10.200.{octet}.2");
        let subnet = format!("10.200.{octet}.0/24");
        let lease = NetworkLease {
            namespace_path: PathBuf::from(format!("/run/netns/{namespace}")),
            namespace: namespace.clone(),
            tap_name: "tap0".to_owned(),
            guest_mac: "06:00:ac:10:00:02".to_owned(),
            guest_address,
            gateway: gateway.clone(),
            host_veth: host_veth.clone(),
            nft_table: nft_table.clone(),
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
        Ok(lease)
    }

    pub async fn delete(&self, lease: &NetworkLease) -> Result<(), RuntimeError> {
        #[cfg(target_os = "linux")]
        {
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
