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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub media_controls: bool,
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
            media_controls: false,
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
    pub sessions: Vec<SessionPeer>,
    pub error: Option<String>,
}

/// Control-plane state, independent of whether loopback is producing PCM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Connected,
    Disconnected,
}
#[derive(Clone, Debug, Serialize)]
pub struct SessionPeer {
    #[serde(flatten)]
    pub peer: Peer,
    pub state: SessionState,
    pub stream_id: u32,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    Previous,
    PlayPause,
    Next,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaNotice {
    action: MediaAction,
    revision: u64,
}
#[derive(Clone, Debug)]
pub struct MediaRequest {
    pub peer: Peer,
    pub action: MediaAction,
    pub stream_id: u32,
    revision: u64,
    at: Instant,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionNotice {
    state: SessionState,
    stream_id: u32,
    revision: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<SessionNotice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media: Option<MediaNotice>,
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
            session: None,
            media: None,
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
            && matches!(
                m.kind.as_str(),
                "probe" | "hello" | "connect" | "bye" | "session" | "media"
            )
            && ((m.kind == "session") == m.session.is_some())
            && (m.kind != "session" || (m.target.is_some() && m.peer.role == Role::Sender))
            && ((m.kind == "media") == m.media.is_some())
            && (m.kind != "media"
                || (m.target.is_some()
                    && m.request.is_some()
                    && m.peer.role == Role::Receiver
                    && m.media.as_ref().is_some_and(|n| n.revision > 0)))
            && m.target.as_ref().is_none_or(|s| valid_id(s))
            && m.request.as_ref().is_none_or(|s| valid_id(s)))
        .then_some(m)
    }
}
enum Command {
    Identity(Identity),
    Probe(SocketAddr),
    Connect(String, String),
    Session(SocketAddr, SessionNotice),
    Media(String, String, MediaNotice),
}
struct Seen {
    peer: Peer,
    at: Instant,
}
#[derive(Default)]
struct Shared {
    snapshot: Snapshot,
    requests: VecDeque<ConnectRequest>,
    media: VecDeque<MediaRequest>,
    sender_session: Option<(SocketAddr, SessionNotice)>,
    session_revision: u64,
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
    /// Only fresh commands for the still-current connected session can leave the queue.
    pub fn take_media_requests(&self) -> Vec<MediaRequest> {
        let mut shared = self.shared.lock().unwrap();
        let current = shared.sender_session.clone();
        let queued: Vec<_> = shared.media.drain(..).collect();
        queued
            .into_iter()
            .filter(|request| {
                current.as_ref().is_some_and(|(address, session)| {
                    session.state == SessionState::Connected
                        && session.revision == request.revision
                        && *address == request.peer.audio_address()
                }) && request.at.elapsed() < Duration::from_secs(1)
                    && shared.snapshot.peers.iter().any(|peer| {
                        peer.identity.id == request.peer.identity.id
                            && peer.address == request.peer.address
                            && peer.identity.ready
                    })
            })
            .collect()
    }
    pub fn media(&self, id: &str, revision: u64, action: MediaAction) -> Result<()> {
        ensure!(valid_id(id), "invalid device identity");
        let state = self.shared.lock().unwrap();
        ensure!(
            state.snapshot.sessions.len() == 1
                && state
                    .snapshot
                    .sessions
                    .iter()
                    .any(|s| s.peer.identity.id == id
                        && s.state == SessionState::Connected
                        && s.peer.identity.media_controls
                        && s.revision == revision),
            "media session is no longer connected"
        );
        self.tx
            .try_send(Command::Media(
                id.into(),
                random_id()?,
                MediaNotice { action, revision },
            ))
            .context("discovery busy")
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
    pub fn announce_session(
        &self,
        receiver: SocketAddr,
        stream_id: u32,
        state: SessionState,
    ) -> Result<()> {
        ensure!(
            receiver.is_ipv4() && receiver.port() != 0,
            "session requires an IPv4 audio endpoint"
        );
        let mut shared = self.shared.lock().unwrap();
        let notice = SessionNotice {
            state,
            stream_id,
            revision: shared.session_revision.saturating_add(1),
        };
        self.tx
            .try_send(Command::Session(receiver, notice.clone()))
            .context("discovery busy")?;
        shared.session_revision = notice.revision;
        shared.sender_session = Some((receiver, notice));
        shared.media.clear();
        Ok(())
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
    let mut outgoing: Option<(SocketAddr, SessionNotice)> = None;
    let mut incoming: HashMap<String, (SessionPeer, u64, Instant)> = HashMap::new();
    let mut media_seen: VecDeque<(String, String, Instant)> = VecDeque::new();
    while !stop.load(Ordering::Acquire) {
        for command in rx.try_iter().take(32) {
            match command {
                Command::Identity(next) => {
                    identity = next;
                    if !identity.ready {
                        incoming.clear();
                    }
                    last = Instant::now() - HEARTBEAT;
                }
                Command::Session(address, notice) => {
                    if identity.role == Role::Sender {
                        outgoing = Some((address, notice));
                        // Retry transitions independently of the next session. A quick
                        // disconnect/reconnect must not leave the old receiver connected.
                        for peer in seen.values() {
                            if let Some(message) = session_message(&identity, &outgoing, &peer.peer)
                                && pending.len() < 16
                            {
                                pending.push((message, peer.peer.address, Instant::now(), 0));
                            }
                        }
                        let _ = send(
                            &direct,
                            &Message::new("probe", identity.clone()),
                            (address.ip(), port).into(),
                        );
                        last = Instant::now() - HEARTBEAT;
                    }
                }
                Command::Probe(address) => {
                    let _ = send(&direct, &Message::new("probe", identity.clone()), address);
                }
                Command::Media(id, request, notice) => {
                    if identity.role == Role::Receiver
                        && identity.ready
                        && incoming
                            .values()
                            .filter(|(session, _, _)| session.state != SessionState::Disconnected)
                            .count()
                            == 1
                        && let Some((session, _, at)) = incoming.get(&id)
                        && at.elapsed() < PEER_TTL
                        && session.state == SessionState::Connected
                        && session.peer.identity.media_controls
                        && session.revision == notice.revision
                        && pending.len() < 16
                    {
                        let mut message = Message::new("media", identity.clone());
                        message.target = Some(id);
                        message.request = Some(request);
                        message.media = Some(notice);
                        pending.push((message, session.peer.address, Instant::now(), 0));
                    }
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
                announce(&direct, &identity, &outgoing, &peer.peer);
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
                // Media controls must arrive on the already discovered receiver's
                // direct endpoint; a media packet cannot introduce or move a peer.
                if message.kind == "media"
                    && seen.get(&message.peer.id).is_none_or(|p| {
                        p.peer.address != address
                            || p.at.elapsed() >= PEER_TTL
                            || p.peer.identity.role != message.peer.role
                            || p.peer.identity.audio_port != message.peer.audio_port
                            || !p.peer.identity.ready
                    })
                {
                    continue;
                }
                if message.kind == "bye" {
                    seen.remove(&message.peer.id);
                    incoming.remove(&message.peer.id);
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
                if matches!(message.kind.as_str(), "probe" | "hello") {
                    announce(&direct, &identity, &outgoing, &peer);
                }
                if message.kind == "session"
                    && identity.role == Role::Receiver
                    && identity.ready
                    && message.target.as_deref() == Some(&identity.id)
                    && let Some(notice) = message.session
                    && incoming
                        .get(&peer.identity.id)
                        .is_none_or(|(_, revision, _)| notice.revision >= *revision)
                {
                    incoming.insert(
                        peer.identity.id.clone(),
                        (
                            SessionPeer {
                                peer: peer.clone(),
                                state: notice.state,
                                stream_id: notice.stream_id,
                                revision: notice.revision,
                            },
                            notice.revision,
                            Instant::now(),
                        ),
                    );
                }
                if message.kind == "media"
                    && identity.role == Role::Sender
                    && identity.media_controls
                    && peer.identity.role == Role::Receiver
                    && peer.identity.ready
                    && message.target.as_deref() == Some(&identity.id)
                    && let (Some(notice), Some(request)) = (message.media, message.request.as_ref())
                    && !media_seen
                        .iter()
                        .any(|(id, key, _)| id == &peer.identity.id && key == request)
                    && let Some((destination, session)) = &outgoing
                    && session.state == SessionState::Connected
                    && session.revision == notice.revision
                    && *destination == peer.audio_address()
                    && session_message(&identity, &outgoing, &peer).is_some()
                {
                    let mut state = shared.lock().unwrap();
                    // announce_session invalidates old commands synchronously, including
                    // the interval before this worker processes its next command batch.
                    if state
                        .sender_session
                        .as_ref()
                        .is_some_and(|(address, current)| {
                            address == destination
                                && current.revision == notice.revision
                                && current.state == SessionState::Connected
                        })
                        && state.media.len() < 16
                    {
                        if media_seen.len() == 128 {
                            media_seen.pop_front();
                        }
                        media_seen.push_back((
                            peer.identity.id.clone(),
                            request.clone(),
                            Instant::now(),
                        ));
                        state.media.push_back(MediaRequest {
                            peer: peer.clone(),
                            action: notice.action,
                            stream_id: session.stream_id,
                            revision: notice.revision,
                            at: Instant::now(),
                        });
                    }
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
        incoming.retain(|id, (_, _, at)| seen.contains_key(id) && at.elapsed() < PEER_TTL);
        requests.retain(|(_, at)| at.elapsed() < Duration::from_secs(10));
        media_seen.retain(|(_, _, at)| at.elapsed() < Duration::from_secs(10));
        let mut peers: Vec<_> = seen.values().map(|p| p.peer.clone()).collect();
        peers.sort_by(|a, b| {
            a.identity
                .name
                .cmp(&b.identity.name)
                .then(a.identity.id.cmp(&b.identity.id))
        });
        let mut sessions: Vec<_> = incoming
            .values()
            .filter(|(session, _, _)| session.state != SessionState::Disconnected)
            .map(|(session, _, _)| session.clone())
            .collect();
        sessions.sort_by(|a, b| a.peer.identity.id.cmp(&b.peer.identity.id));
        let mut state = shared.lock().unwrap();
        state.snapshot.peers = peers;
        state.snapshot.sessions = sessions;
        drop(state);
        thread::sleep(Duration::from_millis(30));
    }
    let message = Message::new("bye", identity);
    for peer in seen.values() {
        let _ = send(&direct, &message, peer.peer.address);
    }
}

fn announce(
    socket: &UdpSocket,
    identity: &Identity,
    outgoing: &Option<(SocketAddr, SessionNotice)>,
    peer: &Peer,
) {
    if let Some(message) = session_message(identity, outgoing, peer) {
        let _ = send(socket, &message, peer.address);
    }
}

fn session_message(
    identity: &Identity,
    outgoing: &Option<(SocketAddr, SessionNotice)>,
    peer: &Peer,
) -> Option<Message> {
    if let Some((address, notice)) = outgoing
        && peer.identity.role == Role::Receiver
        && peer.audio_address() == *address
    {
        let mut message = Message::new("session", identity.clone());
        message.target = Some(peer.identity.id.clone());
        message.session = Some(notice.clone());
        return Some(message);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_is_session_scoped_and_retries_execute_only_once() {
        let mut source = identity(Role::Sender);
        source.media_controls = true;
        let source_id = source.id.clone();
        let sender = Discovery::start_on(source, 0).unwrap();
        let receiver = Discovery::start_on(identity(Role::Receiver), 0).unwrap();
        receiver
            .probe((Ipv4Addr::LOCALHOST, sender.local_addr().port()).into())
            .unwrap();
        wait(|| sender.snapshot().peers.len() == 1 && receiver.snapshot().peers.len() == 1);
        assert!(
            receiver
                .media(&source_id, 1, MediaAction::PlayPause)
                .is_err()
        );
        let destination = (Ipv4Addr::LOCALHOST, 46210).into();
        sender
            .announce_session(destination, 7, SessionState::Connected)
            .unwrap();
        wait(|| receiver.snapshot().sessions.len() == 1);
        let revision = receiver.snapshot().sessions[0].revision;
        receiver
            .media(&source_id, revision, MediaAction::PlayPause)
            .unwrap();
        let mut received = Vec::new();
        wait(|| {
            received.extend(sender.take_media_requests());
            received.len() == 1
        });
        assert_eq!(received[0].action, MediaAction::PlayPause);
        assert_eq!(received[0].stream_id, 7);
        thread::sleep(Duration::from_millis(750));
        assert!(
            sender.take_media_requests().is_empty(),
            "UDP retry must not toggle playback again"
        );
        for action in [MediaAction::Previous, MediaAction::Next] {
            receiver.media(&source_id, revision, action).unwrap();
        }
        received.clear();
        wait(|| {
            received.extend(sender.take_media_requests());
            received.len() == 2
        });
        assert!(received.iter().any(|r| r.action == MediaAction::Previous));
        assert!(received.iter().any(|r| r.action == MediaAction::Next));
        receiver
            .media(&source_id, revision, MediaAction::Next)
            .unwrap();
        wait(|| !sender.shared.lock().unwrap().media.is_empty());
        sender
            .announce_session(destination, 7, SessionState::Disconnected)
            .unwrap();
        assert!(
            sender.take_media_requests().is_empty(),
            "disconnect invalidates queued commands synchronously"
        );
        wait(|| receiver.snapshot().sessions.is_empty());
        assert!(
            receiver
                .media(&source_id, revision, MediaAction::Next)
                .is_err()
        );
        sender
            .announce_session(destination, 7, SessionState::Connected)
            .unwrap();
        wait(|| !receiver.snapshot().sessions.is_empty());
        assert!(
            receiver
                .media(&source_id, revision, MediaAction::Next)
                .is_err(),
            "old notification cannot control a new session"
        );
        let revision = receiver.snapshot().sessions[0].revision;
        receiver
            .media(&source_id, revision, MediaAction::Previous)
            .unwrap();
        wait(|| !sender.take_media_requests().is_empty());
    }

    #[test]
    fn media_rejects_unknown_commands_missing_scope_and_foreign_receiver() {
        let mut source = identity(Role::Sender);
        source.media_controls = true;
        let source_id = source.id.clone();
        let sender = Discovery::start_on(source, 0).unwrap();
        let receiver = identity(Role::Receiver);
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endpoint = (Ipv4Addr::LOCALHOST, sender.local_addr().port()).into();
        send(&socket, &Message::new("probe", receiver.clone()), endpoint).unwrap();
        wait(|| sender.snapshot().peers.len() == 1);
        sender
            .announce_session(
                (Ipv4Addr::LOCALHOST, 46210).into(),
                1,
                SessionState::Connected,
            )
            .unwrap();
        let mut message = Message::new("media", receiver);
        message.target = Some(source_id);
        message.request = Some(random_id().unwrap());
        message.media = Some(MediaNotice {
            action: MediaAction::Next,
            revision: 1,
        });
        let good = serde_json::to_value(&message).unwrap();
        assert!(Message::decode(&serde_json::to_vec(&good).unwrap()).is_some());
        for (field, value) in [
            ("target", serde_json::Value::Null),
            ("request", serde_json::Value::Null),
            (
                "media",
                serde_json::json!({"action":"volume_up","revision":1}),
            ),
            ("media", serde_json::json!({"action":"next","revision":0})),
        ] {
            let mut invalid = good.clone();
            invalid[field] = value;
            assert!(Message::decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        // A new socket cannot impersonate the known receiver by sending a media packet.
        let stranger = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        send(&stranger, &message, endpoint).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(sender.take_media_requests().is_empty());
        assert_eq!(
            sender.snapshot().peers[0].address.port(),
            socket.local_addr().unwrap().port()
        );
        message.peer.audio_port = 46212;
        send(&socket, &message, endpoint).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(
            sender.take_media_requests().is_empty(),
            "another audio destination cannot control this sender"
        );
        message.peer.audio_port = 46210;
        message.target = Some(random_id().unwrap());
        send(&socket, &message, endpoint).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(sender.take_media_requests().is_empty());
    }
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
    fn silent_session_is_visible_without_pcm_and_disconnects() {
        let sender = Discovery::start_on(identity(Role::Sender), 0).unwrap();
        let receiver = Discovery::start_on(identity(Role::Receiver), 0).unwrap();
        let mut other_identity = identity(Role::Receiver);
        other_identity.audio_port = 46212;
        let other = Discovery::start_on(other_identity, 0).unwrap();
        for listener in [&receiver, &other] {
            listener
                .probe((Ipv4Addr::LOCALHOST, sender.local_addr().port()).into())
                .unwrap();
        }
        wait(|| sender.snapshot().peers.len() == 2);
        let destination = (Ipv4Addr::LOCALHOST, 46210).into();
        // Distinct audio endpoints are required to distinguish two receivers on one host.
        sender
            .announce_session(destination, 7, SessionState::Connecting)
            .unwrap();
        wait(|| receiver.snapshot().sessions.len() == 1);
        assert_eq!(
            receiver.snapshot().sessions[0].state,
            SessionState::Connecting
        );
        sender
            .announce_session(destination, 7, SessionState::Connected)
            .unwrap();
        wait(|| receiver.snapshot().sessions[0].state == SessionState::Connected);
        assert_eq!(receiver.snapshot().sessions[0].stream_id, 7);
        assert!(other.snapshot().sessions.is_empty());
        sender
            .announce_session(destination, 7, SessionState::Disconnected)
            .unwrap();
        wait(|| receiver.snapshot().sessions.is_empty());
        sender
            .announce_session(destination, 7, SessionState::Connected)
            .unwrap();
        wait(|| !receiver.snapshot().sessions.is_empty());
        sender
            .announce_session(destination, 7, SessionState::Disconnected)
            .unwrap();
        sender
            .announce_session(
                (Ipv4Addr::LOCALHOST, 46212).into(),
                7,
                SessionState::Connected,
            )
            .unwrap();
        wait(|| receiver.snapshot().sessions.is_empty() && !other.snapshot().sessions.is_empty());
        drop(sender);
        wait(|| other.snapshot().sessions.is_empty());
    }
    #[test]
    fn session_notices_reject_wrong_target_and_older_revision() {
        let id = identity(Role::Receiver);
        let receiver = Discovery::start_on(id.clone(), 0).unwrap();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target = (Ipv4Addr::LOCALHOST, receiver.local_addr().port()).into();
        let mut message = Message::new("session", identity(Role::Sender));
        message.target = Some(random_id().unwrap());
        message.session = Some(SessionNotice {
            state: SessionState::Connected,
            stream_id: 1,
            revision: 2,
        });
        send(&socket, &message, target).unwrap();
        wait(|| !receiver.snapshot().peers.is_empty());
        assert!(receiver.snapshot().sessions.is_empty());
        message.target = Some(id.id);
        send(&socket, &message, target).unwrap();
        wait(|| !receiver.snapshot().sessions.is_empty());
        message.session.as_mut().unwrap().state = SessionState::Disconnected;
        message.session.as_mut().unwrap().revision = 3;
        send(&socket, &message, target).unwrap();
        wait(|| receiver.snapshot().sessions.is_empty());
        message.session.as_mut().unwrap().state = SessionState::Connected;
        message.session.as_mut().unwrap().revision = 2;
        send(&socket, &message, target).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(
            receiver.snapshot().sessions.is_empty(),
            "late datagram must not revive a closed session"
        );
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
