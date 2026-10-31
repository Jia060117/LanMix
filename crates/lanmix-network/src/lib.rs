//! Bounded UDP reception and timestamp-based packet reordering.
//! All allocation and socket operations stay on the network worker.
use anyhow::{Context, Result, ensure};
use lanmix_core::{MasterControls, Mixer, StereoFrame, TrackControls, new_mixer};
use lanmix_proto::{Header, MAX_DATAGRAM_BYTES, decode};
use rtrb::Producer;
use serde::Serialize;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::{BTreeMap, VecDeque},
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub struct ReceiverConfig {
    pub bind: SocketAddr,
    pub buffer_ms: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TrackSnapshot {
    pub slot: usize,
    pub source: String,
    pub stream_id: u32,
    pub session_id: u64,
    pub sample_rate: u32,
    pub online: bool,
    pub packets: u64,
    pub lost_frames: u64,
    pub late_packets: u64,
    pub queue_overflows: u64,
    pub last_seen_ms: u64,
    /// Lifetime diagnostics for this source identity, retained across timeline resets.
    pub reset_count: u64,
    pub reset_reason: String,
    pub concealed_frames: u64,
    pub loss_events: u64,
    #[serde(flatten)]
    pub delivery: DeliveryHealth,
    pub last_loss_age_ms: Option<u64>,
}

/// Source-lifetime evidence; a sequence gap is not proof of Wi-Fi loss:
/// the sender also increments sequence numbers after failed socket sends.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeliveryHealth {
    pub duplicate_packets: u64,
    pub gap_filled_packets: u64,
    pub missing_datagrams: u64,
    pub sender_gap_frames: u64,
    pub transport_gap_frames: u64,
    pub unknown_gap_frames: u64,
    pub last_loss_frames: u64,
    pub last_loss_kind: &'static str,
    #[serde(skip)]
    last_loss_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct NetworkStats {
    pub received_packets: u64,
    pub invalid_packets: u64,
    pub rejected_sources: u64,
    pub received_bytes: u64,
}

#[derive(Default)]
struct Counters {
    packets: AtomicU64,
    invalid: AtomicU64,
    rejected: AtomicU64,
    bytes: AtomicU64,
}

pub struct Receiver {
    pub controls: Vec<Arc<TrackControls>>,
    pub master: Arc<MasterControls>,
    snapshots: Arc<Mutex<Vec<TrackSnapshot>>>,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    local_addr: SocketAddr,
}

impl Receiver {
    pub fn bind(config: ReceiverConfig, output_rate: u32) -> Result<(Self, Mixer)> {
        Self::bind_inner(config, output_rate, false)
    }

    /// UI receivers can keep a newly assigned source silent until its saved
    /// controls have been applied. Reconnected known sources retain controls.
    pub fn bind_with_muted_sources(
        config: ReceiverConfig,
        output_rate: u32,
    ) -> Result<(Self, Mixer)> {
        Self::bind_inner(config, output_rate, true)
    }

    fn bind_inner(
        config: ReceiverConfig,
        output_rate: u32,
        mute_new_sources: bool,
    ) -> Result<(Self, Mixer)> {
        ensure!(
            (20..=200).contains(&config.buffer_ms),
            "buffer must be 20..200 ms"
        );
        ensure!(
            (8000..=192000).contains(&output_rate),
            "unsupported output rate"
        );
        let socket = Socket::new(
            Domain::for_address(config.bind),
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        socket.set_recv_buffer_size(4 * 1024 * 1024)?;
        socket
            .bind(&config.bind.into())
            .with_context(|| format!("cannot bind {}", config.bind))?;
        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let local_addr = socket.local_addr()?;
        let (mixer, producers, controls, master) = new_mixer(output_rate, config.buffer_ms);
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let counters = Arc::new(Counters::default());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let controls = controls.clone();
            let snapshots = snapshots.clone();
            let counters = counters.clone();
            let stop = stop.clone();
            thread::Builder::new()
                .name("lanmix-receive".into())
                .spawn(move || {
                    receive_loop(
                        socket,
                        producers,
                        controls,
                        snapshots,
                        counters,
                        stop,
                        mute_new_sources,
                    );
                })?
        };
        Ok((
            Self {
                controls,
                master,
                snapshots,
                counters,
                stop,
                worker: Some(worker),
                local_addr,
            },
            mixer,
        ))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn snapshot(&self) -> Vec<TrackSnapshot> {
        self.snapshots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn stats(&self) -> NetworkStats {
        NetworkStats {
            received_packets: self.counters.packets.load(Ordering::Relaxed),
            invalid_packets: self.counters.invalid.load(Ordering::Relaxed),
            rejected_sources: self.counters.rejected.load(Ordering::Relaxed),
            received_bytes: self.counters.bytes.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Pending {
    frames: Vec<StereoFrame>,
    arrived: Instant,
    seq: u32,
}

struct Timeline {
    next: Option<u64>,
    pending: BTreeMap<u64, Pending>,
    late: u64,
    lost: u64,
    reorder_wait: Duration,
    previous_seq: Option<u32>,
    consumed: VecDeque<(u64, u32)>,
    delivery: DeliveryHealth,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            next: None,
            pending: BTreeMap::new(),
            late: 0,
            lost: 0,
            reorder_wait: Duration::from_millis(12),
            previous_seq: None,
            consumed: VecDeque::with_capacity(512),
            delivery: DeliveryHealth::default(),
        }
    }
}

impl Timeline {
    fn set_buffer_ms(&mut self, buffer_ms: u32) {
        // Leave playback headroom, but use the reservoir the user actually
        // selected. A fixed 12 ms deadline discards recoverable Wi-Fi packets
        // even when tens/hundreds of milliseconds of audio are still buffered.
        self.reorder_wait =
            Duration::from_millis(buffer_ms.saturating_sub(30).clamp(12, 200) as u64);
    }

    fn is_duplicate(&self, index: u64, seq: u32) -> bool {
        self.pending.get(&index).is_some_and(|p| p.seq == seq)
            || self.consumed.contains(&(index, seq))
    }

    #[cfg(test)]
    fn insert(&mut self, index: u64, frames: Vec<StereoFrame>, now: Instant) {
        self.insert_packet(index, index as u32, frames, now);
    }

    fn insert_packet(&mut self, index: u64, seq: u32, frames: Vec<StereoFrame>, now: Instant) {
        if self.is_duplicate(index, seq) {
            self.delivery.duplicate_packets += 1;
            return;
        }
        let Some(end) = index.checked_add(frames.len() as u64) else {
            self.late += 1;
            return;
        };
        if self.next.is_some_and(|next| index < next) || self.pending.contains_key(&index) {
            self.late += 1;
            return;
        }
        // Reject overlapping ranges: a variable-sized or hostile packet must not rewind time.
        if self
            .pending
            .range(..index)
            .next_back()
            .is_some_and(|(&start, p)| start.saturating_add(p.frames.len() as u64) > index)
            || self
                .pending
                .range(index..)
                .next()
                .is_some_and(|(&start, _)| start < end)
        {
            self.late += 1;
            return;
        }
        if self.next.is_none() {
            self.next = Some(index);
        }
        if self.pending.len() >= 256 {
            if self.next == Some(index) {
                // Preserve the gap-filler that unlocks all queued audio.
                // Sacrifice the farthest future packet to keep memory bounded.
                self.pending.pop_last();
                self.late += 1;
            } else {
                self.late += 1;
                return;
            }
        }
        if self.next == Some(index) && !self.pending.is_empty() {
            self.delivery.gap_filled_packets += 1;
        }
        self.pending.insert(
            index,
            Pending {
                frames,
                arrived: now,
                seq,
            },
        );
    }

    fn drain(&mut self, now: Instant, mut audio: impl FnMut(Option<&[StereoFrame]>, usize)) {
        while let Some(next) = self.next {
            if let Some(packet) = self.pending.remove(&next) {
                let len = packet.frames.len();
                audio(Some(&packet.frames), len);
                self.previous_seq = Some(packet.seq);
                if self.consumed.len() == 512 {
                    self.consumed.pop_front();
                }
                self.consumed.push_back((next, packet.seq));
                self.next = Some(next.saturating_add(len as u64));
                continue;
            }
            let Some((&first, _)) = self.pending.first_key_value() else {
                break;
            };
            // A bounded wait allows reordering, without letting one missing packet stall a stream.
            let gap_discovered = self.pending.values().map(|p| p.arrived).min().unwrap();
            if now.saturating_duration_since(gap_discovered) < self.reorder_wait {
                break;
            }
            let gap = first.saturating_sub(next) as usize;
            if gap > 0 {
                let first_seq = self.pending[&first].seq;
                let distance = self.previous_seq.map(|seq| first_seq.wrapping_sub(seq));
                let kind = match distance {
                    Some(1) => {
                        self.delivery.sender_gap_frames += gap as u64;
                        "sender_timeline"
                    }
                    Some(d) if (2..0x8000_0000).contains(&d) => {
                        self.delivery.missing_datagrams += u64::from(d - 1);
                        self.delivery.transport_gap_frames += gap as u64;
                        "unreceived_datagrams"
                    }
                    _ => {
                        self.delivery.unknown_gap_frames += gap as u64;
                        "unknown_sequence"
                    }
                };
                self.delivery.last_loss_at = Some(now);
                self.delivery.last_loss_frames = gap as u64;
                self.delivery.last_loss_kind = kind;
                audio(None, gap);
                self.lost += gap as u64;
            }
            self.next = Some(first);
        }
    }
}

struct Slot {
    producer: Producer<StereoFrame>,
    key: Option<(IpAddr, u32)>,
    session: u64,
    retired: Vec<u64>,
    rate: u32,
    last_seen: Instant,
    online: bool,
    timeline: Timeline,
    packets: u64,
    overflow: u64,
    last_frame: StereoFrame,
    fade_in: usize,
    resets: u64,
    reset_reason: &'static str,
    concealed_frames: u64,
    loss_events: u64,
}

fn receive_loop(
    socket: UdpSocket,
    producers: Vec<Producer<StereoFrame>>,
    controls: Vec<Arc<TrackControls>>,
    snapshots: Arc<Mutex<Vec<TrackSnapshot>>>,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    mute_new_sources: bool,
) {
    let mut slots: Vec<_> = producers
        .into_iter()
        .map(|producer| Slot {
            producer,
            key: None,
            session: 0,
            retired: Vec::new(),
            rate: 48000,
            last_seen: Instant::now(),
            online: false,
            timeline: Timeline::default(),
            packets: 0,
            overflow: 0,
            last_frame: [0.0; 2],
            fade_in: 0,
            resets: 0,
            reset_reason: "initial",
            concealed_frames: 0,
            loss_events: 0,
        })
        .collect();
    // One extra byte detects oversized/truncated datagrams instead of accepting their prefix.
    let mut bytes = [0u8; MAX_DATAGRAM_BYTES + 1];
    let mut last_snapshot = Instant::now() - Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) {
        match socket.recv_from(&mut bytes) {
            Ok((len, addr)) => {
                counters.packets.fetch_add(1, Ordering::Relaxed);
                counters.bytes.fetch_add(len as u64, Ordering::Relaxed);
                match decode(&bytes[..len]) {
                    Ok(packet) => {
                        let key = (addr.ip(), packet.header.stream_id);
                        let index = slots
                            .iter()
                            .position(|s| s.key == Some(key))
                            .or_else(|| slots.iter().position(|s| s.key.is_none()))
                            .or_else(|| slots.iter().position(|s| !s.online));
                        if let Some(index) = index {
                            if slots[index].key != Some(key) {
                                let control = &controls[index];
                                control.set_muted(mute_new_sources);
                                control.set_solo(false);
                                control.set_gain_db(0.0);
                                control.set_preamp_db(0.0);
                                control.set_balance(0.0);
                                control.set_eq_enabled(false);
                                for band in 0..10 {
                                    control.set_eq_band(band, 0.0);
                                }
                            }
                            handle_packet(
                                &mut slots[index],
                                &controls[index],
                                key,
                                packet.header,
                                packet.frames().collect(),
                            );
                        } else {
                            counters.rejected.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(_) => {
                        counters.invalid.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                counters.invalid.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(5));
            }
        }
        let now = Instant::now();
        for (index, slot) in slots.iter_mut().enumerate() {
            if !slot.online {
                continue;
            }
            let Slot {
                timeline,
                producer,
                overflow,
                last_frame,
                fade_in,
                concealed_frames,
                loss_events,
                ..
            } = slot;
            timeline.set_buffer_ms(controls[index].target_buffer_ms());
            timeline.drain(now, |frames, count| {
                if let Some(frames) = frames {
                    for frame in frames {
                        let mut sample = *frame;
                        if *fade_in > 0 {
                            let gain = (65 - *fade_in) as f32 / 64.0;
                            sample = sample.map(|x| x * gain);
                            *fade_in -= 1;
                        }
                        *last_frame = sample;
                        if producer.push(sample).is_err() {
                            *overflow += 1;
                        }
                    }
                } else {
                    *concealed_frames += count as u64;
                    *loss_events += 1;
                    // Missing time must remain in the stream to preserve its source clock.
                    let previous = *last_frame;
                    for i in 0..count.min(192000) {
                        let gain = 1.0 - ((i + 1).min(64) as f32 / 64.0);
                        if producer.push(previous.map(|x| x * gain)).is_err() {
                            *overflow += 1;
                        }
                    }
                    *last_frame = [0.0; 2];
                    *fade_in = 64;
                }
            });
            if now.duration_since(slot.last_seen) > Duration::from_secs(3) {
                slot.online = false;
                controls[index].set_active(false);
                controls[index].reset();
                let delivery = std::mem::take(&mut slot.timeline.delivery);
                slot.timeline = Timeline {
                    delivery,
                    ..Timeline::default()
                };
                slot.resets += 1;
                slot.reset_reason = "timeout";
            }
        }
        if now.duration_since(last_snapshot) >= Duration::from_millis(80) {
            let view = slots
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| {
                    slot.key.map(|(ip, id)| TrackSnapshot {
                        slot: index,
                        source: ip.to_string(),
                        stream_id: id,
                        session_id: slot.session,
                        sample_rate: slot.rate,
                        online: slot.online,
                        packets: slot.packets,
                        lost_frames: slot.timeline.lost,
                        late_packets: slot.timeline.late,
                        queue_overflows: slot.overflow,
                        last_seen_ms: now.duration_since(slot.last_seen).as_millis() as u64,
                        reset_count: slot.resets,
                        reset_reason: slot.reset_reason.to_owned(),
                        concealed_frames: slot.concealed_frames,
                        loss_events: slot.loss_events,
                        delivery: slot.timeline.delivery.clone(),
                        last_loss_age_ms: slot
                            .timeline
                            .delivery
                            .last_loss_at
                            .map(|at| now.saturating_duration_since(at).as_millis() as u64),
                    })
                })
                .collect();
            *snapshots.lock().unwrap_or_else(|e| e.into_inner()) = view;
            last_snapshot = now;
        }
    }
    for control in controls {
        control.set_active(false);
    }
}

fn handle_packet(
    slot: &mut Slot,
    control: &TrackControls,
    key: (IpAddr, u32),
    header: Header,
    frames: Vec<StereoFrame>,
) {
    let now = Instant::now();
    if slot.key == Some(key) && slot.retired.contains(&header.session_id) {
        return;
    }
    let changed_key = slot.key != Some(key);
    let changed_session = slot.session != header.session_id;
    if !changed_key
        && !changed_session
        && slot.timeline.is_duplicate(header.frame_index, header.seq)
    {
        slot.timeline.delivery.duplicate_packets += 1;
        return;
    }
    if !changed_key
        && !changed_session
        && slot
            .timeline
            .next
            .is_some_and(|next| header.frame_index < next)
    {
        // Validate time before a format change can reset the source clock.
        slot.timeline.late += 1;
        return;
    }
    let discontinuity = slot.timeline.next.is_some_and(|next| {
        header.frame_index > next.saturating_add(header.sample_rate as u64 / 2)
    });
    if changed_key
        || changed_session
        || !slot.online
        || slot.rate != header.sample_rate
        || discontinuity
    {
        if changed_key {
            slot.retired.clear();
            slot.packets = 0;
            slot.overflow = 0;
            slot.resets = 0;
            slot.concealed_frames = 0;
            slot.loss_events = 0;
        } else if changed_session {
            slot.retired.push(slot.session);
            if slot.retired.len() > 8 {
                slot.retired.remove(0);
            }
        }
        slot.resets += 1;
        slot.reset_reason = if changed_key {
            "new_source"
        } else if !slot.online {
            "reconnect"
        } else if changed_session {
            "sender_session"
        } else if slot.rate != header.sample_rate {
            "sample_rate"
        } else {
            "timeline_jump"
        };
        control.set_active(false);
        control.set_sample_rate(header.sample_rate);
        control.reset();
        let delivery = if changed_key {
            DeliveryHealth::default()
        } else {
            std::mem::take(&mut slot.timeline.delivery)
        };
        slot.timeline = Timeline {
            delivery,
            ..Timeline::default()
        };
        slot.key = Some(key);
        slot.session = header.session_id;
        slot.rate = header.sample_rate;
        slot.last_frame = [0.0; 2];
        slot.fade_in = 0;
        control.set_active(true);
    }
    slot.online = true;
    slot.last_seen = now;
    slot.packets += 1;
    slot.timeline
        .insert_packet(header.frame_index, header.seq, frames, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_classifies_sender_gaps_and_unreceived_datagrams_including_wrap() {
        for (before, after, expected_sender, expected_missing) in [
            (10, 11, 96, 0),
            (10, 13, 0, 2),
            (u32::MAX, 0, 96, 0),
            (u32::MAX - 1, 1, 0, 2),
        ] {
            let now = Instant::now();
            let mut timeline = Timeline::default();
            timeline.insert_packet(0, before, vec![[0.25; 2]; 96], now);
            timeline.drain(now, |_, _| {});
            timeline.insert_packet(192, after, vec![[0.5; 2]; 96], now);
            timeline.drain(now + Duration::from_millis(12), |_, _| {});
            assert_eq!(timeline.delivery.sender_gap_frames, expected_sender);
            assert_eq!(timeline.delivery.missing_datagrams, expected_missing);
            assert_eq!(timeline.delivery.transport_gap_frames, 96 - expected_sender);
            assert_eq!(timeline.delivery.last_loss_frames, 96);
        }
    }

    #[test]
    fn delayed_backups_restore_a_three_packet_burst_exactly_once_without_silence() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.set_buffer_ms(150);
        let original: Vec<_> = (0..12)
            .flat_map(|n| vec![[n as f32 / 20.0; 2]; 96])
            .collect();
        let mut output = Vec::new();
        // Drop three originals (6 ms); deliver later originals normally.
        for seq in 0..12u32 {
            if (3..6).contains(&seq) {
                continue;
            }
            let at = now + Duration::from_millis(u64::from(seq) * 2);
            timeline.insert_packet(
                u64::from(seq) * 96,
                seq,
                original[seq as usize * 96..(seq as usize + 1) * 96].to_vec(),
                at,
            );
            timeline.drain(at, |f, _| output.extend_from_slice(f.expect("false loss")));
        }
        for seq in 0..12u32 {
            let at = now + Duration::from_millis(20 + u64::from(seq) * 2);
            timeline.insert_packet(
                u64::from(seq) * 96,
                seq,
                original[seq as usize * 96..(seq as usize + 1) * 96].to_vec(),
                at,
            );
            timeline.drain(at, |f, _| {
                output.extend_from_slice(f.expect("backup missed deadline"))
            });
        }
        assert_eq!(
            output, original,
            "must preserve every PCM sample exactly once"
        );
        assert_eq!(timeline.lost, 0);
        assert_eq!(timeline.late, 0);
        assert_eq!(timeline.delivery.duplicate_packets, 9);
        assert_eq!(timeline.delivery.gap_filled_packets, 3);
    }

    #[test]
    fn both_copies_lost_or_backup_too_late_still_records_real_loss() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.set_buffer_ms(150);
        timeline.insert_packet(0, 0, vec![[0.25; 2]; 96], now);
        timeline.drain(now, |_, _| {});
        timeline.insert_packet(192, 2, vec![[0.25; 2]; 96], now);
        timeline.drain(now + Duration::from_millis(120), |_, _| {});
        assert_eq!(timeline.lost, 96);
        assert_eq!(timeline.delivery.missing_datagrams, 1);
        timeline.insert_packet(96, 1, vec![[0.25; 2]; 96], now + Duration::from_millis(121));
        assert_eq!(timeline.late, 1);
        assert_eq!(timeline.delivery.duplicate_packets, 0);
        assert_eq!(timeline.delivery.gap_filled_packets, 0);
    }

    #[test]
    fn recovered_pending_packet_evicted_at_capacity_can_be_received_again() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert_packet(0, 0, vec![[0.1; 2]], now);
        timeline.drain(now, |_, _| {});
        for index in 2..258u32 {
            timeline.insert_packet(index as u64, index, vec![[0.1; 2]], now);
        }
        timeline.insert_packet(1, 1, vec![[0.1; 2]], now);
        timeline.drain(now, |_, _| {});
        timeline.insert_packet(257, 257, vec![[0.1; 2]], now);
        timeline.drain(now, |_, _| {});
        assert_eq!(timeline.next, Some(258));
        assert_eq!(timeline.lost, 0);
        assert_eq!(timeline.delivery.duplicate_packets, 0);
    }

    #[test]
    fn concealed_network_loss_can_be_silent_without_a_mixer_underrun() {
        let (mut mixer, mut producers, controls, _) = new_mixer(8000, 20);
        controls[0].set_active(true);
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[0.25; 2]; 400], now);
        // Twenty milliseconds really missing; enough later PCM exists to keep
        // the mixer primed. Missing-frame concealment is not queue starvation.
        timeline.insert(560, vec![[0.25; 2]; 400], now);
        timeline.drain(now + Duration::from_secs(1), |frames, count| {
            for index in 0..count {
                producers[0]
                    .push(frames.map_or([0.0; 2], |f| f[index]))
                    .unwrap();
            }
        });
        let mut output = [[0.0; 2]; 80];
        let mut audible_then_silent = false;
        for block in 0..8 {
            mixer.render(&mut output);
            if block == 5 {
                audible_then_silent = output.iter().any(|f| f[0].abs() < 1e-6);
            }
        }
        assert!(
            audible_then_silent,
            "concealed loss should remain visible in PCM"
        );
        assert_eq!(timeline.lost, 160);
        assert_eq!(controls[0].underruns(), 0);
        assert!(controls[0].primed());
    }

    #[test]
    fn selected_buffer_recovers_thirty_millisecond_reordering_without_false_loss() {
        let mut timeline = Timeline::default();
        timeline.set_buffer_ms(70);
        let now = Instant::now();
        timeline.insert(0, vec![[0.1; 2]; 96], now);
        let mut audio = Vec::new();
        timeline.drain(now, |frames, _| audio.extend_from_slice(frames.unwrap()));
        timeline.insert(192, vec![[0.3; 2]; 96], now);
        timeline.drain(now + Duration::from_millis(20), |_, _| {
            panic!("a 70 ms buffer must not conceal a recoverable 20 ms reorder");
        });
        timeline.insert(96, vec![[0.2; 2]; 96], now + Duration::from_millis(30));
        timeline.drain(now + Duration::from_millis(30), |frames, _| {
            audio
                .extend_from_slice(frames.expect("received audio was falsely replaced by silence"));
        });
        assert_eq!(
            audio,
            [vec![[0.1; 2]; 96], vec![[0.2; 2]; 96], vec![[0.3; 2]; 96]].concat()
        );
        assert_eq!(timeline.lost, 0);
        assert_eq!(timeline.late, 0);
    }

    #[test]
    fn buffer_aware_reordering_still_expires_for_real_loss() {
        let mut timeline = Timeline::default();
        timeline.set_buffer_ms(70);
        let now = Instant::now();
        timeline.insert(0, vec![[0.1; 2]; 96], now);
        timeline.drain(now, |_, _| {});
        timeline.insert(192, vec![[0.3; 2]; 96], now);
        timeline.drain(now + Duration::from_millis(39), |_, _| {
            panic!("deadline moved early")
        });
        let mut missing = 0;
        timeline.drain(now + Duration::from_millis(40), |frames, count| {
            if frames.is_none() {
                missing += count;
            }
        });
        assert_eq!(missing, 96);
        assert_eq!(timeline.next, Some(288));
        assert_eq!(timeline.lost, 96);
    }

    #[test]
    fn reordered_packets_preserve_timeline_and_duplicates_are_ignored() {
        let mut timeline = Timeline::default();
        let now = Instant::now();
        timeline.insert(0, vec![[1.0; 2]; 2], now);
        let mut out = Vec::new();
        timeline.drain(now, |f, _| out.extend_from_slice(f.unwrap()));
        timeline.insert(4, vec![[3.0; 2]; 2], now);
        timeline.drain(now, |_, _| panic!("must wait for missing packet"));
        timeline.insert(2, vec![[2.0; 2]; 2], now);
        timeline.insert(2, vec![[9.0; 2]; 2], now);
        timeline.drain(now, |f, _| out.extend_from_slice(f.unwrap()));
        assert_eq!(
            out,
            vec![[1.0; 2], [1.0; 2], [2.0; 2], [2.0; 2], [3.0; 2], [3.0; 2]]
        );
        assert_eq!(timeline.late, 0);
        assert_eq!(timeline.delivery.duplicate_packets, 1);
    }

    #[test]
    fn missing_packet_gets_exact_silence_after_deadline() {
        let mut timeline = Timeline::default();
        let now = Instant::now();
        timeline.insert(0, vec![[1.0; 2]; 2], now);
        timeline.drain(now, |_, _| {});
        timeline.insert(5, vec![[2.0; 2]; 2], now);
        let mut silence = 0;
        timeline.drain(now + Duration::from_millis(13), |f, count| {
            if f.is_none() {
                silence += count;
            }
        });
        assert_eq!(silence, 3);
        assert_eq!(timeline.lost, 3);
        assert_eq!(timeline.next, Some(7));
    }

    #[test]
    fn overlapping_packets_cannot_rewind_or_replace_pending_audio() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[1.0; 2]; 4], now);
        timeline.insert(8, vec![[2.0; 2]; 4], now);
        // Overlap the right and left edges of the pending [8, 12) range.
        timeline.insert(10, vec![[9.0; 2]; 4], now);
        timeline.insert(6, vec![[9.0; 2]; 4], now);
        timeline.insert(4, vec![[3.0; 2]; 4], now);
        let mut output = Vec::new();
        timeline.drain(now, |frames, _| output.extend_from_slice(frames.unwrap()));
        assert_eq!(
            output,
            [vec![[1.0; 2]; 4], vec![[3.0; 2]; 4], vec![[2.0; 2]; 4]].concat()
        );
        assert_eq!(timeline.next, Some(12));
        assert_eq!(timeline.lost, 0);
        assert_eq!(timeline.late, 2);
        // Even a packet whose tail is new must not replay consumed frames.
        timeline.insert(10, vec![[9.0; 2]; 4], now);
        timeline.drain(now, |_, _| panic!("partially late audio was replayed"));
        assert_eq!(timeline.next, Some(12));
        assert_eq!(timeline.late, 3);
    }

    #[test]
    fn gap_wait_expires_at_exactly_twelve_milliseconds() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[1.0; 2]; 2], now);
        timeline.drain(now, |_, _| {});
        timeline.insert(4, vec![[2.0; 2]; 2], now);
        timeline.drain(
            now + Duration::from_millis(12) - Duration::from_nanos(1),
            |_, _| {
                panic!("reordering deadline has not elapsed");
            },
        );
        let mut events = Vec::new();
        timeline.drain(now + Duration::from_millis(12), |frames, count| {
            events.push((frames.is_some(), count));
        });
        assert_eq!(events, [(false, 2), (true, 2)]);
        assert_eq!(timeline.next, Some(6));
        assert_eq!(timeline.lost, 2);
    }

    #[test]
    fn nearer_reordered_packet_does_not_restart_existing_gap_deadline() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[1.0; 2]; 2], now);
        timeline.drain(now, |_, _| {});
        timeline.insert(6, vec![[3.0; 2]; 2], now);
        timeline.insert(4, vec![[2.0; 2]; 2], now + Duration::from_millis(11));
        let mut events = Vec::new();
        timeline.drain(now + Duration::from_millis(12), |frames, count| {
            events.push((frames.is_some(), count));
        });
        // The gap at [2, 4) has already waited 12 ms, even though index 4
        // arrived recently. Reordering must not keep moving its deadline.
        assert_eq!(events, [(false, 2), (true, 2), (true, 2)]);
        assert_eq!(timeline.next, Some(8));
        assert_eq!(timeline.lost, 2);
    }

    #[test]
    fn full_pending_queue_still_accepts_the_packet_at_the_playback_cursor() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[1.0; 2]], now);
        timeline.drain(now, |_, _| {});
        for index in 2..258 {
            timeline.insert(index, vec![[index as f32; 2]], now);
        }
        assert_eq!(timeline.pending.len(), 256);
        timeline.insert(1, vec![[1.0; 2]], now);
        assert!(
            timeline.pending.len() <= 256,
            "pending capacity must remain bounded"
        );
        let mut output = Vec::new();
        timeline.drain(now, |frames, _| output.extend_from_slice(frames.unwrap()));
        assert_eq!(
            output.first(),
            Some(&[1.0; 2]),
            "received gap-filler was dropped at capacity"
        );
        assert!(
            output.len() >= 256,
            "contiguous buffered packets did not drain"
        );
        assert_eq!(
            timeline.lost, 0,
            "received frames must not be concealed as lost"
        );
        assert!(timeline.pending.is_empty());
    }

    #[test]
    fn pending_queue_rejects_excess_future_packets_without_evicting_audio() {
        let now = Instant::now();
        let mut timeline = Timeline::default();
        timeline.insert(0, vec![[1.0; 2]], now);
        timeline.drain(now, |_, _| {});
        for index in 2..258 {
            timeline.insert(index, vec![[index as f32; 2]], now);
        }
        timeline.insert(258, vec![[9.0; 2]], now);
        assert_eq!(timeline.pending.len(), 256);
        assert!(!timeline.pending.contains_key(&258));
        let mut audio_frames = 0;
        let mut missing_frames = 0;
        timeline.drain(now + Duration::from_millis(12), |frames, count| {
            if frames.is_some() {
                audio_frames += count;
            } else {
                missing_frames += count;
            }
        });
        assert_eq!(audio_frames, 256);
        assert_eq!(missing_frames, 1);
        assert_eq!(timeline.next, Some(258));
    }

    fn slot_fixture() -> (Slot, Arc<TrackControls>, Mixer) {
        let (mixer, mut producers, controls, _) = new_mixer(48_000, 20);
        let slot = Slot {
            producer: producers.remove(0),
            key: None,
            session: 0,
            retired: Vec::new(),
            rate: 48_000,
            last_seen: Instant::now(),
            online: false,
            timeline: Timeline::default(),
            packets: 0,
            overflow: 0,
            last_frame: [0.0; 2],
            fade_in: 0,
            resets: 0,
            reset_reason: "initial",
            concealed_frames: 0,
            loss_events: 0,
        };
        (slot, Arc::clone(&controls[0]), mixer)
    }

    fn deliver(slot: &mut Slot, control: &TrackControls, session: u64, index: u64, rate: u32) {
        let header = Header {
            session_id: session,
            stream_id: 7,
            seq: 0,
            frame_index: index,
            sample_rate: rate,
            frame_count: 2,
        };
        // Exercise the actual codec-to-network path, rather than manually
        // constructing mismatched headers and payloads.
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        let len = lanmix_proto::encode(&header, &[[session as f32; 2]; 2], &mut bytes).unwrap();
        let packet = decode(&bytes[..len]).unwrap();
        handle_packet(
            slot,
            control,
            (IpAddr::from([127, 0, 0, 1]), 7),
            packet.header,
            packet.frames().collect(),
        );
    }

    #[test]
    fn retired_session_packet_cannot_reset_or_keep_alive_current_session() {
        let (mut slot, control, _mixer) = slot_fixture();
        deliver(&mut slot, &control, 10, 0, 48_000);
        deliver(&mut slot, &control, 11, 100, 48_000);
        let seen = slot.last_seen;
        let packets = slot.packets;
        slot.last_frame = [0.5; 2];
        slot.fade_in = 17;
        deliver(&mut slot, &control, 10, 1_000_000, 192_000);
        assert_eq!(slot.session, 11);
        assert_eq!(slot.rate, 48_000);
        assert_eq!(control.sample_rate(), 48_000);
        assert_eq!(slot.timeline.next, Some(100));
        assert_eq!(slot.timeline.pending.len(), 1);
        assert_eq!(slot.last_seen, seen);
        assert_eq!(slot.packets, packets);
        assert_eq!(slot.last_frame, [0.5; 2]);
        assert_eq!(slot.fade_in, 17);
        assert!(control.active());
    }

    #[test]
    fn forward_discontinuity_drops_old_pending_audio_without_a_huge_gap() {
        let (mut slot, control, _mixer) = slot_fixture();
        deliver(&mut slot, &control, 10, 0, 48_000);
        deliver(&mut slot, &control, 10, 10, 48_000);
        deliver(&mut slot, &control, 10, 1_000_000, 48_000);
        // A delayed packet from before the jump must not restore old time.
        deliver(&mut slot, &control, 10, 2, 48_000);
        let mut output = Vec::new();
        slot.timeline
            .drain(Instant::now() + Duration::from_secs(1), |frames, _| {
                output.extend_from_slice(
                    frames.expect("discontinuity must reset instead of concealing a huge gap"),
                );
            });
        assert_eq!(output.len(), 2);
        assert_eq!(slot.timeline.next, Some(1_000_002));
        assert_eq!(slot.timeline.lost, 0);
        assert_eq!(slot.timeline.late, 1);
        assert_eq!(slot.resets, 2);
        assert_eq!(slot.reset_reason, "timeline_jump");
    }

    #[test]
    fn delayed_packet_from_previous_rate_cannot_rewind_current_session() {
        let (mut slot, control, _mixer) = slot_fixture();
        deliver(&mut slot, &control, 10, 0, 48_000);
        slot.timeline.drain(Instant::now(), |_, _| {});
        deliver(&mut slot, &control, 10, 100, 96_000);
        slot.timeline.drain(Instant::now(), |_, _| {});
        // Still the same session, but this packet belongs to the old rate
        // and its entire timestamp range precedes the active cursor.
        deliver(&mut slot, &control, 10, 2, 48_000);
        assert_eq!(
            slot.rate, 96_000,
            "stale packet rolled the sample rate backward"
        );
        assert_eq!(control.sample_rate(), 96_000);
        assert_eq!(slot.timeline.next, Some(102));
        assert!(slot.timeline.pending.is_empty());
    }

    fn send_test_datagram(receiver: &Receiver, stream: u32, session: u64, index: u64) {
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let header = Header {
            session_id: session,
            stream_id: stream,
            seq: 0,
            frame_index: index,
            sample_rate: 48_000,
            frame_count: 2,
        };
        let mut bytes = [0; MAX_DATAGRAM_BYTES];
        let len = lanmix_proto::encode(&header, &[[0.25; 2]; 2], &mut bytes).unwrap();
        assert_eq!(
            sender
                .send_to(&bytes[..len], receiver.local_addr())
                .unwrap(),
            len
        );
    }

    fn wait_for_track(
        receiver: &Receiver,
        predicate: impl Fn(&TrackSnapshot) -> bool,
    ) -> TrackSnapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = receiver.snapshot();
            if let Some(track) = snapshot.iter().find(|track| predicate(track)) {
                return track.clone();
            }
            assert!(
                Instant::now() < deadline,
                "receiver did not reach expected state: {snapshot:?}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn customize_control(control: &TrackControls) {
        control.set_muted(false);
        control.set_solo(true);
        control.set_gain_db(-9.0);
        control.set_preamp_db(-3.0);
        control.set_balance(0.25);
        control.set_eq_enabled(true);
        for band in 0..10 {
            control.set_eq_band(band, band as f32 - 5.0);
        }
    }

    fn assert_custom_control(control: &TrackControls) {
        assert!(!control.muted());
        assert!(control.solo());
        assert_eq!(control.gain_db(), -9.0);
        assert_eq!(control.preamp_db(), -3.0);
        assert_eq!(control.balance(), 0.25);
        assert!(control.eq_enabled());
        for band in 0..10 {
            assert_eq!(control.eq_band(band), band as f32 - 5.0);
        }
    }

    #[test]
    fn ui_mode_mutes_new_sources_and_preserves_controls_on_known_source_reconnect() {
        let config = ReceiverConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            buffer_ms: 20,
        };
        let (receiver, _mixer) = Receiver::bind_with_muted_sources(config, 48_000).unwrap();
        send_test_datagram(&receiver, 7, 10, 0);
        let first = wait_for_track(&receiver, |track| track.stream_id == 7 && track.online);
        let control = &receiver.controls[first.slot];
        assert!(control.muted(), "new UI source must await saved settings");
        customize_control(control);
        send_test_datagram(&receiver, 7, 10, 2);
        wait_for_track(&receiver, |track| {
            track.stream_id == 7 && track.packets >= 2
        });
        assert_custom_control(control);
        send_test_datagram(&receiver, 7, 11, 0);
        wait_for_track(&receiver, |track| {
            track.stream_id == 7 && track.session_id == 11
        });
        assert_custom_control(control);
        wait_for_track(&receiver, |track| track.stream_id == 7 && !track.online);
        send_test_datagram(&receiver, 7, 12, 0);
        let reconnected = wait_for_track(&receiver, |track| track.session_id == 12 && track.online);
        assert_eq!(reconnected.slot, first.slot);
        assert!(reconnected.reset_count >= 4);
        assert_eq!(reconnected.reset_reason, "reconnect");
        assert_custom_control(control);
    }

    // Restrict the real receive loop to one slot to exercise slot replacement
    // without fabricating routing or copying the production reset logic.
    fn one_slot_receiver(mute_new_sources: bool) -> (Receiver, Mixer) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(5)))
            .unwrap();
        let (mixer, mut producers, mut controls, master) = new_mixer(48_000, 20);
        producers.truncate(1);
        controls.truncate(1);
        let mut receiver = Receiver {
            controls,
            master,
            snapshots: Arc::new(Mutex::new(Vec::new())),
            counters: Arc::new(Counters::default()),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            local_addr: socket.local_addr().unwrap(),
        };
        let controls = receiver.controls.clone();
        let snapshots = Arc::clone(&receiver.snapshots);
        let counters = Arc::clone(&receiver.counters);
        let stop = Arc::clone(&receiver.stop);
        receiver.worker = Some(thread::spawn(move || {
            receive_loop(
                socket,
                producers,
                controls,
                snapshots,
                counters,
                stop,
                mute_new_sources,
            );
        }));
        (receiver, mixer)
    }

    #[test]
    fn reused_slot_resets_all_source_controls_and_applies_receiver_mute_policy() {
        for mute_new_sources in [false, true] {
            let (receiver, _mixer) = one_slot_receiver(mute_new_sources);
            send_test_datagram(&receiver, 7, 10, 0);
            wait_for_track(&receiver, |track| track.stream_id == 7 && track.online);
            let control = &receiver.controls[0];
            assert_eq!(control.muted(), mute_new_sources);
            customize_control(control);
            wait_for_track(&receiver, |track| track.stream_id == 7 && !track.online);
            send_test_datagram(&receiver, 8, 11, 0);
            let replacement =
                wait_for_track(&receiver, |track| track.stream_id == 8 && track.online);
            assert_eq!(replacement.slot, 0);
            assert_eq!(replacement.packets, 1);
            assert_eq!(control.muted(), mute_new_sources);
            assert!(!control.solo());
            assert_eq!(control.gain_db(), 0.0);
            assert_eq!(control.preamp_db(), 0.0);
            assert_eq!(control.balance(), 0.0);
            assert!(!control.eq_enabled());
            for band in 0..10 {
                assert_eq!(control.eq_band(band), 0.0);
            }
        }
    }
}
