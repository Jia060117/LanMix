#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use anyhow::Result;
use clap::Parser;
use eframe::egui::{self, Color32, RichText};
use lanmix_audio::{OutputDeviceInfo, OutputStream, output_devices, prepare_output};
use lanmix_core::{EQ_FREQUENCIES, MasterControls, TrackControls};
use lanmix_discovery::{Discovery, Identity, Role};
use lanmix_network::{Receiver, ReceiverConfig};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(91, 224, 182);

#[derive(Parser)]
#[command(version, about = "LanMix multi-device audio mixer")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:46210")]
    bind: SocketAddr,
    /// Run the real receiver and mixer without opening an audio device or window.
    #[arg(long)]
    headless: bool,
    #[arg(long, default_value_t = 10)]
    seconds: u64,
    #[arg(long)]
    report: Option<PathBuf>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct MixSettings {
    gain_db: f32,
    muted: bool,
    solo: bool,
    balance: f32,
    eq_enabled: bool,
    preamp_db: f32,
    bands: [f32; 10],
}
impl Default for MixSettings {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            muted: false,
            solo: false,
            balance: 0.0,
            eq_enabled: false,
            preamp_db: 0.0,
            bands: [0.0; 10],
        }
    }
}
impl MixSettings {
    fn apply_track(&self, control: &TrackControls) {
        control.set_gain_db(self.gain_db);
        control.set_solo(self.solo);
        control.set_balance(self.balance);
        control.set_eq_enabled(self.eq_enabled);
        control.set_preamp_db(self.preamp_db);
        for (i, &db) in self.bands.iter().enumerate() {
            control.set_eq_band(i, db);
        }
        // New sources remain gated until all saved tone controls are published.
        control.set_muted(self.muted);
    }
    fn apply_master(&self, control: &MasterControls) {
        control.set_gain_db(self.gain_db);
        control.set_eq_enabled(self.eq_enabled);
        control.set_preamp_db(self.preamp_db);
        for (i, &db) in self.bands.iter().enumerate() {
            control.set_eq_band(i, db);
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    output_name: Option<String>,
    sample_rate: u32,
    buffer_ms: u32,
    master: MixSettings,
    tracks: HashMap<String, MixSettings>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            output_name: None,
            sample_rate: 0,
            buffer_ms: 150,
            master: MixSettings {
                gain_db: -6.0,
                ..Default::default()
            },
            tracks: HashMap::new(),
        }
    }
}

fn settings_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("LanMix").join("settings.json")
}

struct Engine {
    stream: OutputStream,
    receiver: Receiver,
    name: String,
    rate: u32,
    channels: u16,
    format: String,
}

struct App {
    bind: SocketAddr,
    settings: Settings,
    devices: Vec<OutputDeviceInfo>,
    engine: Option<Engine>,
    error: Option<String>,
    notice: Option<String>,
    applied: HashSet<String>,
    last_refresh: Instant,
    retry_default: bool,
    discovery: Option<Discovery>,
    identity: Identity,
    discovery_host: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, bind: SocketAddr) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        for path in [
            "C:/Windows/Fonts/msyh.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert(
                    "system-cjk".into(),
                    egui::FontData::from_owned(bytes).into(),
                );
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts
                        .families
                        .entry(family)
                        .or_default()
                        .push("system-cjk".into());
                }
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(17, 23, 29);
        visuals.window_fill = Color32::from_rgb(24, 32, 40);
        visuals.selection.bg_fill = Color32::from_rgb(37, 91, 76);
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        cc.egui_ctx.set_style(style);
        let settings = std::fs::read(settings_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "LanMix PC".into());
        let mut identity = Identity::new(name, Role::Receiver, env!("CARGO_PKG_VERSION"))
            .expect("OS random identity");
        identity.audio_port = bind.port();
        let discovery = Discovery::start(identity.clone());
        let discovery_error = discovery
            .as_ref()
            .err()
            .map(|e| format!("自动发现不可用：{e:#}；仍可按 IP 连接"));
        let mut app = Self {
            bind,
            settings,
            devices: Vec::new(),
            engine: None,
            error: discovery_error,
            notice: None,
            applied: HashSet::new(),
            last_refresh: Instant::now(),
            retry_default: false,
            discovery: discovery.ok(),
            identity,
            discovery_host: String::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        match output_devices() {
            Ok(devices) => self.devices = devices,
            Err(e) => self.error = Some(format!("Device query: {e:#}")),
        }
        self.last_refresh = Instant::now();
    }

    fn start(&mut self) {
        self.retry_default = false;
        self.engine = None;
        self.applied.clear();
        match self.build_engine() {
            Ok(engine) => {
                self.engine = Some(engine);
                self.error = None;
                self.notice = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn build_engine(&self) -> Result<Engine> {
        let setup = prepare_output(
            self.settings.output_name.as_deref(),
            (self.settings.sample_rate != 0).then_some(self.settings.sample_rate),
        )?;
        let (receiver, mixer) = Receiver::bind_with_muted_sources(
            ReceiverConfig {
                bind: self.bind,
                buffer_ms: self.settings.buffer_ms,
            },
            setup.sample_rate,
        )?;
        self.settings.master.apply_master(&receiver.master);
        let name = setup.device_name.clone();
        let rate = setup.sample_rate;
        let channels = setup.channels;
        let format = setup.sample_format.clone();
        let stream = setup.start(mixer)?;
        Ok(Engine {
            stream,
            receiver,
            name,
            rate,
            channels,
            format,
        })
    }

    fn publish(&mut self) {
        let ready = self.engine.is_some();
        if ready != self.identity.ready {
            self.identity.ready = ready;
            if let Some(discovery) = &self.discovery {
                let _ = discovery.update(self.identity.clone());
            }
        }
    }

    fn nearby(&mut self, ui: &mut egui::Ui) {
        ui.heading("附近的设备");
        ui.small("同一局域网 · 自动刷新");
        let Some(discovery) = &self.discovery else {
            ui.small("自动发现不可用，仍可使用原来的 IP 连接。");
            return;
        };
        let snapshot = discovery.snapshot();
        if snapshot.peers.is_empty() {
            ui.small("正在寻找设备…请在电脑打开 LanMix 发送窗口。");
        }
        let mut connect = None;
        for peer in snapshot.peers {
            ui.group(|ui| {
                ui.strong(&peer.identity.name);
                ui.small(format!(
                    "{} · {}",
                    peer.address.ip(),
                    if peer.identity.role == Role::Sender {
                        "发送端"
                    } else {
                        "接收端"
                    }
                ));
                if peer.identity.role == Role::Sender
                    && ui
                        .add_enabled(
                            peer.identity.ready,
                            egui::Button::new(if peer.identity.ready {
                                "连接并接收"
                            } else {
                                "正在使用或未开放连接"
                            }),
                        )
                        .clicked()
                {
                    connect = Some(peer.identity.id.clone());
                }
            });
        }
        if let Some(error) = snapshot.error {
            ui.small(error);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.discovery_host)
                    .hint_text("电脑 IPv4 地址")
                    .desired_width(165.0),
            );
            if ui.small_button("查找").clicked() {
                match self.discovery_host.trim().parse::<std::net::Ipv4Addr>() {
                    Ok(ip) => {
                        if let Err(e) = discovery.probe((ip, lanmix_discovery::PORT).into()) {
                            self.error = Some(e.to_string());
                        }
                    }
                    Err(_) => self.error = Some("请输入有效的 IPv4 地址".into()),
                }
            }
        });
        if let Some(id) = connect {
            if self.engine.is_none() {
                self.start();
            }
            self.publish();
            if self.engine.is_some()
                && let Some(discovery) = &self.discovery
            {
                match discovery.connect(&id) {
                    Ok(()) => self.notice = Some("已发出连接请求，等待电脑音源…".into()),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
    }

    fn save(&mut self) {
        let result = (|| -> Result<()> {
            let path = settings_path();
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, serde_json::to_vec_pretty(&self.settings)?)?;
            Ok(())
        })();
        match result {
            Ok(()) => self.notice = Some("混音已保存到本机".into()),
            Err(e) => self.error = Some(format!("Save failed: {e:#}")),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_refresh.elapsed() > Duration::from_secs(3) {
            self.refresh();
            let default_changed = self.engine.as_ref().is_some_and(|engine| {
                self.settings.output_name.is_none()
                    && self
                        .devices
                        .iter()
                        .find(|d| d.is_default)
                        .is_some_and(|d| d.name != engine.name)
            });
            if self.settings.output_name.is_some() {
                self.retry_default = false;
            }
            if default_changed || self.retry_default {
                let should_retry = default_changed || self.retry_default;
                self.start();
                self.retry_default = should_retry && self.engine.is_none();
            }
        }
        let audio_error = self.engine.as_ref().and_then(|e| e.stream.take_error());
        if let Some(error) = audio_error {
            self.engine = None;
            self.retry_default = self.settings.output_name.is_none();
            self.error = Some(format!(
                "Output disconnected: {error}. {}",
                if self.retry_default {
                    "Waiting for a system default output."
                } else {
                    "Select an output and restart."
                }
            ));
        }
        self.publish();
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("LanMix").size(30.0).strong().color(ACCENT));
                ui.label(
                    RichText::new("多个设备，一个声音出口。")
                        .size(11.0)
                        .color(Color32::GRAY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("保存混音").clicked() {
                        self.save();
                    }
                    if self.engine.is_some() || self.retry_default {
                        if ui.button("停止接收").clicked() {
                            self.engine = None;
                            self.retry_default = false;
                        }
                        ui.label(
                            RichText::new(if self.engine.is_some() {
                                "● 正在接收"
                            } else {
                                "○ 等待输出"
                            })
                            .color(ACCENT),
                        );
                    } else if ui.button(RichText::new("开始接收").strong()).clicked() {
                        self.start();
                    }
                });
            });
            ui.add_space(10.0);
        });
        egui::SidePanel::left("routing")
            .exact_width(290.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("routing_scroll")
                    .show(ui, |ui| {
                        ui.add_space(12.0);
                        self.nearby(ui);
                        ui.add_space(12.0);
                        ui.separator();
                        ui.heading("声音输出");
                        ui.label("将所有音源汇入一个输出设备。");
                        ui.add_space(8.0);
                        egui::ComboBox::from_id_salt("device")
                            .width(260.0)
                            .selected_text(
                                self.settings
                                    .output_name
                                    .as_deref()
                                    .unwrap_or("跟随系统默认输出"),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings.output_name,
                                    None,
                                    "跟随系统默认输出",
                                );
                                for device in &self.devices {
                                    ui.selectable_value(
                                        &mut self.settings.output_name,
                                        Some(device.name.clone()),
                                        &device.name,
                                    );
                                }
                            });
                        ui.horizontal(|ui| {
                            if ui.small_button("刷新").clicked() {
                                self.refresh();
                            }
                            if self.engine.is_some() && ui.small_button("应用并重启").clicked()
                            {
                                self.start();
                            }
                        });
                        ui.separator();
                        ui.label("应用输出采样率");
                        egui::ComboBox::from_id_salt("rate")
                            .selected_text(if self.settings.sample_rate == 0 {
                                "自动（设备默认）".into()
                            } else {
                                format!("{} Hz", self.settings.sample_rate)
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings.sample_rate,
                                    0,
                                    "自动（设备默认）",
                                );
                                for rate in [44100, 48000, 88200, 96000, 176400, 192000] {
                                    ui.selectable_value(
                                        &mut self.settings.sample_rate,
                                        rate,
                                        format!("{rate} Hz"),
                                    );
                                }
                            });
                        ui.add(
                            egui::Slider::new(&mut self.settings.buffer_ms, 20..=200)
                                .text("初始缓冲 ms"),
                        );
                        ui.small("格式和缓冲调整在重启接收后生效。");
                        ui.separator();
                        ui.label(RichText::new("实际应用输出").size(11.0).color(ACCENT));
                        if let Some(engine) = &self.engine {
                            ui.label(&engine.name);
                            ui.label(format!(
                                "{} Hz · {} ch · {}",
                                engine.rate, engine.channels, engine.format
                            ));
                        } else {
                            ui.label("尚未打开");
                        }
                        ui.small("应用使用系统共享输出；硬件和蓝牙实际格式可能不同。");
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new("系统报告的音频能力").show(ui, |ui| {
                            let selected = self.devices.iter().find(|d| {
                                self.settings
                                    .output_name
                                    .as_ref()
                                    .map_or(d.is_default, |name| name == &d.name)
                            });
                            if let Some(device) = selected {
                                ui.small(format!(
                                    "Default: {} Hz, {} ch, {}",
                                    device.default_rate,
                                    device.default_channels,
                                    device.default_format
                                ));
                                for c in &device.capabilities {
                                    ui.small(format!(
                                        "{} · {} ch · {}–{} Hz",
                                        c.format, c.channels, c.min_rate, c.max_rate
                                    ));
                                }
                            } else {
                                ui.small("设备尚未报告能力");
                            }
                        });
                        ui.separator();
                        ui.label(RichText::new("接收状态").size(11.0).color(ACCENT));
                        ui.label(format!("UDP {}", self.bind));
                        ui.small("最多 16 路 · 双声道 float PCM");
                        if let Some(engine) = &self.engine {
                            let stats = engine.receiver.stats();
                            ui.small(format!(
                                "{} packets · {:.2} MB received",
                                stats.received_packets,
                                stats.received_bytes as f64 / 1_000_000.0
                            ));
                            ui.small(format!(
                                "Invalid {} · full slots {}",
                                stats.invalid_packets, stats.rejected_sources
                            ));
                        }
                    });
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 149, 136), error);
            } else if let Some(notice) = &self.notice {
                ui.colored_label(ACCENT, notice);
            } else {
                ui.small("音质优先 · 每路独立音量与 EQ · 退出时保存设置");
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(12.0);
                ui.heading("总混音");
                let master = self.engine.as_ref().map(|e| e.receiver.master.clone());
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.add(egui::Slider::new(&mut self.settings.master.gain_db, -60.0..=6.0).text("总音量 dB"));
                    if let Some(master) = &master {
                        meter(ui, master.peak());
                        let clipped = master.clipped_frames();
                        if clipped > 0 { ui.colored_label(Color32::YELLOW, format!("Output ceiling engaged: {clipped} frames. Reduce preamp or gain for more headroom.")); }
                    }
                    equalizer(ui, "master_eq", &mut self.settings.master, self.engine.as_ref().map_or(48000, |e| e.rate));
                });
                if let Some(master) = master { self.settings.master.apply_master(&master); }
                ui.add_space(12.0);
                ui.heading("独立音源");
                let tracks = self.engine.as_ref().map(|e| e.receiver.snapshot()).unwrap_or_default();
                if tracks.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("让声音从这里汇合").size(22.0));
                    ui.label("在左侧连接电脑，也可以从电脑发送到本机：");
                    ui.code(format!("lanmix-sender send --to RECEIVER_IP:{} --stream-id 1", self.bind.port()));
                    ui.label("本地测试音命令：");
                    ui.code(format!("lanmix-sender tone --to 127.0.0.1:{} --seconds 10", self.bind.port()));
                    ui.small("同一个 IP 的多个发送实例请使用不同的 stream-id。");
                }
                for track in tracks {
                    let key = format!("{}/{}", track.source, track.stream_id);
                    let mix = self.settings.tracks.entry(key.clone()).or_default();
                    let control: Arc<TrackControls> = self.engine.as_ref().unwrap().receiver.controls[track.slot].clone();
                    self.applied.insert(key.clone());
                    ui.push_id(&key, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(if track.online { "●" } else { "○" }).color(if track.online { ACCENT } else { Color32::GRAY }));
                                let source_name = self.discovery.as_ref().and_then(|d| d.snapshot().peers.into_iter()
                                    .find(|p| p.identity.role == Role::Sender && track.source.parse::<std::net::IpAddr>().is_ok_and(|source| p.address.ip() == source)))
                                    .map(|p| p.identity.name).unwrap_or_else(|| format!("音源 {}", track.slot + 1));
                                ui.label(RichText::new(source_name).size(17.0).strong());
                                ui.small(format!("{} / 流 {}", track.source, track.stream_id));
                                ui.label(format!("{} Hz · {}", track.sample_rate, if track.online { "online" } else { "offline" }));
                            });
                            ui.horizontal(|ui| {
                                ui.add(egui::Slider::new(&mut mix.gain_db, -60.0..=12.0).text("dB"));
                                ui.toggle_value(&mut mix.muted, "静音");
                                ui.toggle_value(&mut mix.solo, "独听");
                                ui.add(egui::Slider::new(&mut mix.balance, -1.0..=1.0).text("L / R"));
                            });
                            meter(ui, control.peak());
                            equalizer(ui, "eq", mix, self.engine.as_ref().unwrap().rate);
                            ui.small(format!("Packets {} · missing {} frames · late {} · duplicates {} · gap-filled {} · overflow {} · underruns {}", track.packets, track.lost_frames, track.late_packets, track.delivery.duplicate_packets, track.delivery.gap_filled_packets, track.queue_overflows, control.underruns()));
                            ui.small(format!("Buffer {:.1} / target {} ms · {} · clock {:+.0} ppm",
                                control.buffered_frames() as f64 * 1000.0 / track.sample_rate as f64,
                                control.target_buffer_ms(), if control.primed() { "playing" } else { "buffering" }, control.drift_ppm()));
                        });
                    });
                    mix.apply_track(&control);
                    ui.add_space(6.0);
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }
}

fn meter(ui: &mut egui::Ui, peak: f32) {
    let db = if peak > 0.000001 {
        20.0 * peak.log10()
    } else {
        -120.0
    };
    ui.add(
        egui::ProgressBar::new(((db + 60.0) / 60.0).clamp(0.0, 1.0))
            .fill(if db > -1.0 {
                Color32::from_rgb(244, 177, 91)
            } else {
                ACCENT
            })
            .text(if db <= -100.0 {
                "静音".into()
            } else {
                format!("{db:.1} dBFS")
            })
            .desired_height(13.0),
    );
}

fn equalizer(ui: &mut egui::Ui, id: &str, mix: &mut MixSettings, sample_rate: u32) {
    egui::CollapsingHeader::new("10 段均衡器")
        .id_salt(id)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut mix.eq_enabled, "启用");
                ui.add(egui::Slider::new(&mut mix.preamp_db, -24.0..=12.0).text("预放大 dB"));
                if ui.small_button("重置").clicked() {
                    mix.bands = [0.0; 10];
                    mix.preamp_db = 0.0;
                }
            });
            ui.horizontal(|ui| {
                for (i, &frequency) in EQ_FREQUENCIES.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.add_enabled(
                            frequency < sample_rate as f32 * 0.49,
                            egui::Slider::new(&mut mix.bands[i], -12.0..=12.0)
                                .vertical()
                                .show_value(true),
                        );
                        ui.small(if frequency >= 1000.0 {
                            format!("{}k", frequency / 1000.0)
                        } else {
                            format!("{frequency:.0}")
                        });
                    });
                }
            });
        });
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.headless {
        return headless(args);
    }
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LanMix — Network Audio Mixer",
        native_options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, args.bind)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn headless(args: Args) -> Result<()> {
    let (receiver, mut mixer) = Receiver::bind(
        ReceiverConfig {
            bind: args.bind,
            buffer_ms: 60,
        },
        48000,
    )?;
    receiver.master.set_gain_db(-6.0);
    let mut identity = Identity::new(
        "LanMix headless".into(),
        Role::Receiver,
        env!("CARGO_PKG_VERSION"),
    )?;
    identity.audio_port = receiver.local_addr().port();
    identity.ready = true;
    let _discovery = Discovery::start(identity).ok();
    println!(
        "Headless mixer listening at {} for {} seconds",
        receiver.local_addr(),
        args.seconds
    );
    let start = Instant::now();
    let mut frames = [[0.0f32; 2]; 480];
    let mut peak = 0.0f32;
    let mut energy = 0.0f64;
    let mut count = 0u64;
    let mut non_silent = 0u64;
    while start.elapsed() < Duration::from_secs(args.seconds) {
        mixer.render(&mut frames);
        for frame in frames {
            for sample in frame {
                anyhow::ensure!(
                    sample.is_finite() && sample.abs() <= 1.0,
                    "invalid rendered output"
                );
                peak = peak.max(sample.abs());
                energy += (sample as f64).powi(2);
                count += 1;
                if sample.abs() > 0.00001 {
                    non_silent += 1;
                }
            }
        }
        let deadline = start + Duration::from_secs_f64(count as f64 / 96000.0);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let meters: Vec<_> = receiver.controls.iter().enumerate().map(|(slot, control)| {
        serde_json::json!({"slot":slot,"underruns":control.underruns(),"buffered_frames":control.buffered_frames(),
            "target_buffer_ms":control.target_buffer_ms(),"primed":control.primed(),"drift_ppm":control.drift_ppm()})
    }).collect();
    let report = serde_json::json!({ "network":receiver.stats(), "tracks":receiver.snapshot(), "meters":meters, "output":{ "sample_rate":48000,"peak":peak,"rms":(energy / count.max(1) as f64).sqrt(),"non_silent_samples":non_silent,"rendered_samples":count,"clipped_frames":receiver.master.clipped_frames() } });
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, &json)?;
    }
    println!("{json}");
    Ok(())
}
