//! Best-effort IPv4 LAN discovery, completely separate from the PCM path.
//! No authentication: intended for the same trusted LAN as protocol v1.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::{HashMap, VecDeque},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const PORT: u16 = 46211;
const LIMIT: usize = 1024;
const PEER_TTL: Duration = Duration::from_secs(7);
const HEARTBEAT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sender,
    Receiver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub audio_port: u16,
    pub ready: bool,
    pub version: String,
}
impl Identity {
    pub fn new(name: String, role: Role, version: &str) -> Result<Self> {
        Ok(Self {
            id: random_id()?,
            name: clean_name(&name),
            role,
            audio_port: 0,
            ready: false,
            version: version.into(),
        })
    }
    fn valid(&self) -> bool {
        valid_id(&self.id)
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 64
            && !self.name.chars().any(char::is_control)
            && self.version.len() <= 32
            && (self.role != Role::Receiver || !self.ready || self.audio_port != 0)
    }
}

pub fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("device identity: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn clean_name(name: &str) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    if name.trim().is_empty() {
        "LanMix device".into()
    } else {
        name
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Serialize)]
pub struct Peer {
    #[serde(flatten)]
    pub identity: Identity,
    pub address: SocketAddr,
}
impl Peer {
    pub fn audio_address(&self) -> SocketAddr {
        SocketAddr::new(self.address.ip(), self.identity.audio_port)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ConnectRequest {
    pub peer: Peer,
}
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub peers: Vec<Peer>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    magic: String,
    protocol: u8,
    kind: String,
    peer: Identity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<String>,
}
impl Message {
    fn new(kind: &str, peer: Identity) -> Self {
        Self {
            magic: "LMXD".into(),
            protocol: 1,
            kind: kind.into(),
            peer,
            target: None,
            request: None,
        }
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > LIMIT {
            return None;
        }
        let m: Self = serde_json::from_slice(bytes).ok()?;
        (m.magic == "LMXD"
            && m.protocol == 1
            && m.peer.valid()
            && matches!(m.kind.as_str(), "probe" | "hello" | "connect" | "bye")
            && m.target.as_ref().is_none_or(|s| valid_id(s))
            && m.request.as_ref().is_none_or(|s| valid_id(s)))
        .then_some(m)
    }
}
enum Command {
    Identity(Identity),
    Probe(SocketAddr),
    Connect(String, String),
}
struct Seen {
    peer: Peer,
    at: Instant,
}
#[derive(Default)]
struct Shared {
    snapshot: Snapshot,
    requests: VecDeque<ConnectRequest>,
}

pub struct Discovery {
    tx: mpsc::SyncSender<Command>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    address: SocketAddr,
}
impl Discovery {
    pub fn start(identity: Identity) -> Result<Self> {
        Self::start_on(identity, PORT)
    }
    /// Port 0 isolates integration tests; `local_addr` accepts direct probes.
    pub fn start_on(identity: Identity, port: u16) -> Result<Self> {
        ensure!(identity.valid(), "invalid discovery identity");
        let broadcast = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        broadcast.set_reuse_address(true)?;
        broadcast.set_broadcast(true)?;
        broadcast
            .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
            .context("discovery UDP listener")?;
        broadcast.set_nonblocking(true)?;
        let broadcast: UdpSocket = broadcast.into();
        let direct = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        direct.set_broadcast(true)?;
        direct.set_nonblocking(true)?;
        let address = direct.local_addr()?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(32);
        let state = shared.clone();
        let halt = stop.clone();
        let worker = thread::Builder::new()
            .name("lanmix-discovery".into())
            .spawn(move || {
                run(identity, port, broadcast, direct, state, halt, rx);
            })?;
        Ok(Self {
            tx,
            shared,
            stop,
            worker: Some(worker),
            address,
        })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap().snapshot.clone()
    }
    pub fn take_requests(&self) -> Vec<ConnectRequest> {
        self.shared.lock().unwrap().requests.drain(..).collect()
    }
    pub fn update(&self, identity: Identity) -> Result<()> {
        ensure!(identity.valid(), "invalid discovery identity");
        self.tx
            .try_send(Command::Identity(identity))
            .context("discovery busy")
    }
    pub fn probe(&self, address: SocketAddr) -> Result<()> {
        ensure!(
            address.is_ipv4() && address.port() != 0,
            "discovery requires an IPv4 address and port"
        );
        self.tx
            .try_send(Command::Probe(address))
            .context("discovery busy")
    }
    pub fn connect(&self, id: &str) -> Result<()> {
        ensure!(valid_id(id), "invalid device identity");
        self.tx
            .try_send(Command::Connect(id.into(), random_id()?))
            .context("discovery busy")
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn destinations(port: u16) -> Vec<SocketAddr> {
    let mut ips = vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST];
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            if let if_addrs::IfAddr::V4(v4) = interface.addr
                && let Some(ip) = v4.broadcast
            {
                ips.push(ip);
            }
        }
    }
    ips.sort_unstable();
    ips.dedup();
    ips.into_iter().map(|ip| (ip, port).into()).collect()
}
fn send(socket: &UdpSocket, message: &Message, address: SocketAddr) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(message)?;
    socket.send_to(&bytes, address).map(|_| ())
}
fn run(
    mut identity: Identity,
    port: u16,
    broadcast: UdpSocket,
    direct: UdpSocket,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Command>,
) {
    let mut seen: HashMap<String, Seen> = HashMap::new();
    let mut requests: VecDeque<(String, Instant)> = VecDeque::new();
    let mut pending: Vec<(Message, SocketAddr, Instant, u8)> = Vec::new();
    let mut last = Instant::now() - HEARTBEAT;
    let mut buffer = [0; LIMIT + 1];
    while !stop.load(Ordering::Acquire) {
        for command in rx.try_iter().take(32) {
            match command {
                Command::Identity(next) => {
                    identity = next;
                    last = Instant::now() - HEARTBEAT;
                }
                Command::Probe(address) => {
                    let _ = send(&direct, &Message::new("probe", identity.clone()), address);
                }
                Command::Connect(id, request) => {
                    if let Some(peer) = seen.get(&id).filter(|p| p.at.elapsed() < PEER_TTL)
                        && identity.role == Role::Receiver
                        && identity.ready
                        && peer.peer.identity.role == Role::Sender
                    {
                        let mut message = Message::new("connect", identity.clone());
                        message.target = Some(id);
                        message.request = Some(request);
                        if pending.len() < 16 {
                            pending.push((message, peer.peer.address, Instant::now(), 0));
                        }
                    }
                }
            }
        }
        if last.elapsed() >= HEARTBEAT {
            let mut delivered = false;
            let mut failure = None;
            for address in destinations(port) {
                match send(&direct, &Message::new("probe", identity.clone()), address) {
                    Ok(()) => delivered = true,
                    Err(e) => failure = Some(format!("Discovery send: {e}")),
                }
            }
            shared.lock().unwrap().snapshot.error = if delivered { None } else { failure };
            // Direct heartbeats retain discovered peers if a Wi-Fi AP suppresses broadcasts later.
            for peer in seen.values() {
                let _ = send(
                    &direct,
                    &Message::new("hello", identity.clone()),
                    peer.peer.address,
                );
            }
            last = Instant::now();
        }
        for (message, address, due, tries) in &mut pending {
            if Instant::now() >= *due {
                let _ = send(&direct, message, *address);
                *tries += 1;
                *due = Instant::now() + Duration::from_millis(250);
            }
        }
        pending.retain(|(_, _, _, tries)| *tries < 3);
        for socket in [&broadcast, &direct] {
            // Bound packet processing so malformed/flooded traffic cannot starve shutdown/commands.
            for _ in 0..64 {
                let (len, address) = match socket.recv_from(&mut buffer) {
                    Ok(packet) => packet,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        shared.lock().unwrap().snapshot.error = Some(e.to_string());
                        break;
                    }
                };
                let Some(message) = Message::decode(&buffer[..len]) else {
                    continue;
                };
                if message.peer.id == identity.id
                    || !address.is_ipv4()
                    || address.ip().is_unspecified()
                    || address.ip().is_multicast()
                {
                    continue;
                }
                if message.kind == "bye" {
                    seen.remove(&message.peer.id);
                    continue;
                }
                if !seen.contains_key(&message.peer.id) && seen.len() >= 64 {
                    continue;
                }
                let peer = Peer {
                    identity: message.peer,
                    address,
                };
                seen.insert(
                    peer.identity.id.clone(),
                    Seen {
                        peer: peer.clone(),
                        at: Instant::now(),
                    },
                );
                if message.kind == "probe" {
                    let _ = send(&direct, &Message::new("hello", identity.clone()), address);
                }
                if message.kind == "connect"
                    && identity.role == Role::Sender
                    && identity.ready
                    && peer.identity.role == Role::Receiver
                    && peer.identity.ready
                    && message.target.as_deref() == Some(&identity.id)
                    && let Some(request) = message.request
                    && !requests.iter().any(|(id, _)| id == &request)
                {
                    if requests.len() == 64 {
                        requests.pop_front();
                    }
                    requests.push_back((request, Instant::now()));
                    let mut state = shared.lock().unwrap();
                    if state.requests.len() < 16 {
                        state.requests.push_back(ConnectRequest { peer });
                    }
                }
            }
        }
        seen.retain(|_, peer| peer.at.elapsed() < PEER_TTL);
        requests.retain(|(_, at)| at.elapsed() < Duration::from_secs(10));
        let mut peers: Vec<_> = seen.values().map(|p| p.peer.clone()).collect();
        peers.sort_by(|a, b| {
            a.identity
                .name
                .cmp(&b.identity.name)
                .then(a.identity.id.cmp(&b.identity.id))
        });
        shared.lock().unwrap().snapshot.peers = peers;
        thread::sleep(Duration::from_millis(30));
    }
    let message = Message::new("bye", identity);
    for peer in seen.values() {
        let _ = send(&direct, &message, peer.peer.address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(role: Role) -> Identity {
        let mut id = Identity::new("test 电脑".into(), role, "test").unwrap();
        id.audio_port = 46210;
        id.ready = true;
        id
    }
    fn wait(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < deadline, "discovery timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn rejects_invalid_and_foreign_packets() {
        let good = Message::new("hello", identity(Role::Receiver));
        let mut value = serde_json::to_value(good).unwrap();
        assert!(Message::decode(&serde_json::to_vec(&value).unwrap()).is_some());
        for (key, invalid) in [
            ("protocol", serde_json::json!(9)),
            ("magic", serde_json::json!("LMXA")),
            ("kind", serde_json::json!("execute")),
        ] {
            let mut bad = value.clone();
            bad[key] = invalid;
            assert!(Message::decode(&serde_json::to_vec(&bad).unwrap()).is_none());
        }
        value["peer"]["name"] = serde_json::json!("injected\nline");
        assert!(Message::decode(&serde_json::to_vec(&value).unwrap()).is_none());
        assert!(Message::decode(&[0; LIMIT + 1]).is_none());
    }
    #[test]
    fn bidirectional_probe_request_dedup_and_goodbye() {
        let sender = Discovery::start_on(identity(Role::Sender), 0).unwrap();
        let receiver = Discovery::start_on(identity(Role::Receiver), 0).unwrap();
        receiver
            .probe((Ipv4Addr::LOCALHOST, sender.local_addr().port()).into())
            .unwrap();
        wait(|| sender.snapshot().peers.len() == 1 && receiver.snapshot().peers.len() == 1);
        let peer = receiver.snapshot().peers[0].clone();
        receiver.connect(&peer.identity.id).unwrap();
        let mut received = Vec::new();
        wait(|| {
            received.extend(sender.take_requests());
            !received.is_empty()
        });
        thread::sleep(Duration::from_millis(650));
        received.extend(sender.take_requests());
        assert_eq!(received.len(), 1, "retries must not restart audio");
        assert_eq!(received[0].peer.audio_address().port(), 46210);
        drop(receiver);
        wait(|| sender.snapshot().peers.is_empty());
    }
    #[test]
    fn receiver_requests_require_ready_sender() {
        let mut id = identity(Role::Sender);
        id.ready = false;
        let sender = Discovery::start_on(id, 0).unwrap();
        let receiver = Discovery::start_on(identity(Role::Receiver), 0).unwrap();
        receiver
            .probe((Ipv4Addr::LOCALHOST, sender.local_addr().port()).into())
            .unwrap();
        wait(|| !receiver.snapshot().peers.is_empty());
        receiver
            .connect(&receiver.snapshot().peers[0].identity.id)
            .unwrap();
        thread::sleep(Duration::from_millis(800));
        assert!(sender.take_requests().is_empty());
    }
    #[test]
    fn stale_peer_expires_and_malformed_requests_cannot_queue_capture() {
        let sender_id = identity(Role::Sender);
        let sender = Discovery::start_on(sender_id.clone(), 0).unwrap();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, sender.local_addr().port()));
        let mut message = Message::new("connect", identity(Role::Receiver));
        message.target = Some(random_id().unwrap()); // wrong target
        message.request = Some(random_id().unwrap());
        send(&socket, &message, target).unwrap();
        wait(|| !sender.snapshot().peers.is_empty());
        assert!(sender.take_requests().is_empty());
        message.target = Some(sender_id.id);
        message.peer.ready = false; // a stopped receiver must never start capture
        send(&socket, &message, target).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(sender.take_requests().is_empty());
        let deadline = Instant::now() + PEER_TTL + Duration::from_secs(1);
        while !sender.snapshot().peers.is_empty() {
            assert!(Instant::now() < deadline, "crashed peer did not expire");
            thread::sleep(Duration::from_millis(40));
        }
    }
}
