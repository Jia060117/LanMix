//! Native Rust/Win32 sender. UI and discovery never run on the audio callback.
use super::*;
use lanmix_discovery::{Discovery, Identity, Peer, Role, SessionState};
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::mpsc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const LIST: usize = 101;
const CONNECT: usize = 102;
const MANUAL: usize = 103;
const STOP: usize = 104;
const ALLOW: usize = 105;
const OUTPUT: usize = 106;
const REFRESH: usize = 107;
const ADDRESS_CONNECT: usize = 108;
const RESTORE: usize = 301;
const RECONNECT: usize = 302;
const EXIT: usize = 303;
const SYSTEM_EXIT: usize = 0x1ff0;
const PEER_MENU: usize = 1000;
const TRAY: u32 = WM_APP + 1;
const SHOW: u32 = WM_APP + 2;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const CLASS: &str = "LanMixSenderWindow";
const BG: u32 = rgb(246, 248, 252);
const INK: u32 = rgb(29, 38, 55);
const MUTED: u32 = rgb(111, 123, 144);
const BLUE: u32 = rgb(62, 103, 222);
const GREEN: u32 = rgb(25, 135, 101);
const RED: u32 = rgb(190, 61, 68);
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn copy_wide<const N: usize>(dest: &mut [u16; N], s: &str) {
    let value = wide(s);
    let mut count = value.len().min(N - 1);
    if count > 0 && (0xD800..=0xDBFF).contains(&value[count - 1]) {
        count -= 1;
    }
    dest[..count].copy_from_slice(&value[..count]);
    dest[count] = 0;
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Target {
    address: SocketAddr,
    name: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Preferences {
    last: Option<Target>,
    allow: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            last: None,
            allow: true,
        }
    }
}
impl Preferences {
    fn path() -> Option<std::path::PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| std::path::PathBuf::from(p).join("LanMix/sender.json"))
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let Some(path) = Self::path() {
            let _ = (|| -> Result<()> {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, serde_json::to_vec(self)?)?;
                Ok(())
            })();
        }
    }
}
enum Event {
    Ready,
    Finished(std::result::Result<(), String>),
}
struct Session {
    target: Target,
    running: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    events: mpsc::Receiver<Event>,
    ready: bool,
    stopping: bool,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct ButtonFeedback {
    hover: bool,
    amount: f32,
    animated: bool,
}
unsafe extern "system" fn button_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Box remains alive until after its child HWND is destroyed.
    unsafe {
        let feedback = &mut *(data as *mut ButtonFeedback);
        match msg {
            WM_MOUSEMOVE => {
                if !feedback.hover {
                    feedback.hover = true;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                    if feedback.animated {
                        SetTimer(hwnd, 2, 16, None);
                    } else {
                        feedback.amount = 1.0;
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            WM_MOUSELEAVE => {
                feedback.hover = false;
                if feedback.animated {
                    SetTimer(hwnd, 2, 16, None);
                } else {
                    feedback.amount = 0.0;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            WM_TIMER if w == 2 => {
                let target = if feedback.hover { 1.0 } else { 0.0 };
                feedback.amount += (target - feedback.amount) * 0.35;
                if (feedback.amount - target).abs() < 0.02 {
                    feedback.amount = target;
                    KillTimer(hwnd, 2);
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            WM_ENABLE => {
                feedback.hover = false;
                feedback.amount = 0.0;
                KillTimer(hwnd, 2);
            }
            WM_NCDESTROY => {
                KillTimer(hwnd, 2);
                RemoveWindowSubclass(hwnd, Some(button_proc), 1);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, w, l)
    }
}
struct Window {
    hwnd: HWND,
    identity: Identity,
    discovery: Option<Discovery>,
    peers: Vec<Peer>,
    signature: String,
    session: Option<Session>,
    controls: Vec<(usize, HWND)>,
    #[allow(clippy::vec_box)] // Win32 subclass pointers must remain stable as Vec grows.
    feedback: Vec<Box<ButtonFeedback>>,
    font: HFONT,
    small_font: HFONT,
    title_font: HFONT,
    background: HBRUSH,
    white: HBRUSH,
    status_color: u32,
    dpi: f32,
    outputs: Vec<usize>,
    preferences: Preferences,
    icon: HICON,
    tray: bool,
    taskbar_created: u32,
    refresh_until: Option<Instant>,
    hidden_notice: bool,
}
impl Window {
    fn control(&self, id: usize) -> HWND {
        self.controls
            .iter()
            .find(|(key, _)| *key == id)
            .map_or(null_mut(), |(_, handle)| *handle)
    }
    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.dpi).round() as i32
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT {
            left: self.px(x),
            top: self.px(y),
            right: self.px(x + w),
            bottom: self.px(y + h),
        }
    }
    fn status(&mut self, message: &str, color: u32) {
        self.status_color = color;
        unsafe {
            SetWindowTextW(self.control(200), wide(message).as_ptr());
            InvalidateRect(self.control(200), null(), 1);
            InvalidateRect(self.hwnd, &self.rect(24, 96, 552, 102), 0);
        }
        self.update_tray();
    }
    unsafe fn add(
        &mut self,
        class: &str,
        label: &str,
        id: usize,
        rect: (i32, i32, i32, i32),
        extra: u32,
    ) -> Result<()> {
        unsafe {
            let (x, y, w, h) = rect;
            let handle = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | extra,
                self.px(x),
                self.px(y),
                self.px(w),
                self.px(h),
                self.hwnd,
                id as *mut c_void,
                GetModuleHandleW(null()),
                null(),
            );
            ensure!(!handle.is_null(), "create native control");
            SendMessageW(
                handle,
                WM_SETFONT,
                if id == 201 {
                    self.title_font
                } else if [202, 206, 209].contains(&id) {
                    self.small_font
                } else {
                    self.font
                } as usize,
                1,
            );
            self.controls.push((id, handle));
            if class == "BUTTON" && id != ALLOW {
                let mut animated = 1i32;
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    (&mut animated as *mut i32).cast(),
                    0,
                );
                let mut feedback = Box::new(ButtonFeedback {
                    hover: false,
                    amount: 0.0,
                    animated: animated != 0,
                });
                ensure!(
                    SetWindowSubclass(
                        handle,
                        Some(button_proc),
                        1,
                        (&mut *feedback as *mut ButtonFeedback) as usize
                    ) != 0,
                    "button feedback"
                );
                self.feedback.push(feedback);
            }
            Ok(())
        }
    }
    unsafe fn button(&mut self, label: &str, id: usize, rect: (i32, i32, i32, i32)) -> Result<()> {
        unsafe { self.add("BUTTON", label, id, rect, WS_TABSTOP | BS_OWNERDRAW as u32) }
    }
    unsafe fn init(&mut self) -> Result<()> {
        unsafe {
            self.add("STATIC", "LanMix", 201, (24, 18, 250, 42), 0)?;
            self.add("STATIC", "让声音汇合", 202, (24, 64, 400, 22), 0)?;
            self.add("STATIC", "连接状态", 208, (44, 109, 490, 24), 0)?;
            self.add(
                "STATIC",
                "选择附近设备，或输入接收地址。",
                200,
                (44, 141, 494, 42),
                0,
            )?;
            self.add("STATIC", "附近的接收设备", 203, (24, 218, 400, 24), 0)?;
            self.button("刷新设备", REFRESH, (460, 208, 116, 38))?;
            self.add(
                "LISTBOX",
                "",
                LIST,
                (24, 254, 552, 132),
                WS_VSCROLL
                    | WS_TABSTOP
                    | LBS_NOTIFY as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32
                    | LBS_NOINTEGRALHEIGHT as u32,
            )?;
            SendMessageW(
                self.control(LIST),
                LB_SETITEMHEIGHT,
                0,
                self.px(62) as isize,
            );
            self.add(
                "STATIC",
                "正在查找…  请在手机打开 LanMix 并开始接收。",
                209,
                (40, 295, 520, 44),
                0,
            )?;
            self.button("连接设备", CONNECT, (24, 398, 360, 42))?;
            self.button("断开连接", STOP, (396, 398, 180, 42))?;
            self.add("STATIC", "手动连接", 204, (24, 462, 300, 23), 0)?;
            let manual = self
                .preferences
                .last
                .as_ref()
                .map(|t| t.address.to_string())
                .unwrap_or_default();
            self.add(
                "EDIT",
                &manual,
                MANUAL,
                (24, 494, 360, 36),
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            )?;
            SendMessageW(
                self.control(MANUAL),
                EM_SETCUEBANNER,
                0,
                wide("192.168.1.184:46210").as_ptr() as isize,
            );
            self.button("按地址连接", ADDRESS_CONNECT, (396, 490, 180, 42))?;
            self.add("STATIC", "采集输出", 205, (24, 556, 100, 26), 0)?;
            self.add(
                "COMBOBOX",
                "",
                OUTPUT,
                (132, 551, 444, 220),
                WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            )?;
            SendMessageW(
                self.control(OUTPUT),
                CB_ADDSTRING,
                0,
                wide("系统默认输出 · 回环采集").as_ptr() as isize,
            );
            match lanmix_audio::output_devices() {
                Ok(devices) => {
                    for device in devices {
                        SendMessageW(
                            self.control(OUTPUT),
                            CB_ADDSTRING,
                            0,
                            wide(&device.name).as_ptr() as isize,
                        );
                        self.outputs.push(device.index);
                    }
                }
                Err(error) => self.status(&format!("音频设备查询失败：{error:#}"), RED),
            }
            SendMessageW(self.control(OUTPUT), CB_SETCURSEL, 0, 0);
            self.add(
                "BUTTON",
                "允许接收端发起连接（空闲时）",
                ALLOW,
                (24, 598, 552, 30),
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            )?;
            SendMessageW(
                self.control(ALLOW),
                BM_SETCHECK,
                if self.preferences.allow {
                    BST_CHECKED as usize
                } else {
                    0
                },
                0,
            );
            self.add(
                "STATIC",
                "关闭 / 最小化会收起到托盘，声音继续。右键托盘可退出。",
                206,
                (24, 649, 552, 24),
                0,
            )?;
            SetTimer(self.hwnd, 1, 100, None);
            self.sync_controls();
            self.publish();
            Ok(())
        }
    }
    fn begin(&mut self, address: SocketAddr, name: &str) {
        if self.session.is_some() {
            return;
        }
        let selected = unsafe { SendMessageW(self.control(OUTPUT), CB_GETCURSEL, 0, 0) };
        let output = if selected > 0 {
            self.outputs.get(selected as usize - 1).copied()
        } else {
            None
        };
        let options = CaptureOptions {
            to: address,
            output,
            input: None,
            stream_id: 1,
            packet_ms: 2.0,
            repair_ms: 20,
            seconds: 0,
        };
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let (tx, events) = mpsc::channel();
        announce(&self.discovery, address, 1, SessionState::Connecting);
        match thread::Builder::new()
            .name("lanmix-capture".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| {
                    send_observed(options, flag, || {
                        let _ = tx.send(Event::Ready);
                    })
                })
                .map_err(|_| "音频线程异常退出".to_string())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(Event::Finished(result));
            }) {
            Ok(worker) => {
                self.session = Some(Session {
                    target: Target {
                        address,
                        name: name.into(),
                    },
                    running,
                    worker: Some(worker),
                    events,
                    ready: false,
                    stopping: false,
                });
                self.status(
                    &format!("正在连接 {name}…\r\n正在打开系统音频采集，请稍候。"),
                    BLUE,
                );
            }
            Err(e) => {
                announce(&self.discovery, address, 1, SessionState::Disconnected);
                self.status(&format!("无法启动采集：{e}"), RED);
            }
        }
        self.sync_controls();
        self.publish();
    }
    fn stop(&mut self) {
        if let Some(session) = &mut self.session {
            if session.stopping {
                return;
            }
            session.stopping = true;
            session.running.store(false, Ordering::Release);
            announce(
                &self.discovery,
                session.target.address,
                1,
                SessionState::Disconnected,
            );
            self.status("正在断开…\r\n正在释放音频采集，稍后即可重新连接。", MUTED);
            self.sync_controls();
        }
    }
    fn sync_controls(&self) {
        unsafe {
            let idle = self.session.is_none();
            let selected = SendMessageW(self.control(LIST), LB_GETCURSEL, 0, 0);
            let ready = usize::try_from(selected)
                .ok()
                .and_then(|i| self.peers.get(i))
                .is_some_and(|p| p.identity.ready);
            EnableWindow(self.control(CONNECT), i32::from(idle && ready));
            EnableWindow(
                self.control(STOP),
                i32::from(self.session.as_ref().is_some_and(|s| !s.stopping)),
            );
            for id in [OUTPUT, MANUAL, ADDRESS_CONNECT] {
                EnableWindow(self.control(id), i32::from(idle));
            }
            SetWindowTextW(
                self.control(CONNECT),
                wide(if idle {
                    "连接设备"
                } else if self.session.as_ref().is_some_and(|s| s.stopping) {
                    "正在断开…"
                } else if self.session.as_ref().is_some_and(|s| s.ready) {
                    "正在发送"
                } else {
                    "正在连接…"
                })
                .as_ptr(),
            );
        }
    }
    fn publish(&mut self) {
        let allow =
            unsafe { SendMessageW(self.control(ALLOW), BM_GETCHECK, 0, 0) } == BST_CHECKED as isize;
        self.identity.ready = allow && self.session.is_none();
        if let Some(discovery) = &self.discovery {
            let _ = discovery.update(self.identity.clone());
        }
    }
    fn tick(&mut self) {
        loop {
            match self.session.as_ref().and_then(|s| s.events.try_recv().ok()) {
                Some(Event::Ready) => {
                    let session = self.session.as_mut().unwrap();
                    if session.stopping {
                        continue;
                    }
                    session.ready = true;
                    let target = session.target.clone();
                    announce(&self.discovery, target.address, 1, SessionState::Connected);
                    self.preferences.last = Some(target.clone());
                    self.preferences.save();
                    self.status(
                        &format!(
                            "正在发送 → {}\r\n{} · 电脑本地声音保持原样",
                            target.name, target.address
                        ),
                        GREEN,
                    );
                    self.sync_controls();
                }
                Some(Event::Finished(result)) => {
                    if let Some(session) = self.session.take() {
                        announce(
                            &self.discovery,
                            session.target.address,
                            1,
                            SessionState::Disconnected,
                        );
                    }
                    match result {
                        Ok(()) => {
                            self.status("已断开连接\r\n选择设备，或从托盘菜单重新连接。", MUTED)
                        }
                        Err(e) => {
                            self.status(&format!("发送失败：{e}"), RED);
                            self.balloon("发送未能继续", &e);
                        }
                    }
                    self.sync_controls();
                    self.publish();
                }
                None => break,
            }
        }
        if self
            .refresh_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.refresh_until = None;
            unsafe {
                SetWindowTextW(self.control(REFRESH), wide("刷新设备").as_ptr());
                EnableWindow(self.control(REFRESH), 1);
            }
            if self.session.is_none() {
                if self.peers.is_empty() {
                    self.status(
                        "暂未发现接收设备\r\n确认两端处于同一局域网，或使用下方地址连接。",
                        MUTED,
                    );
                } else {
                    self.status(
                        &format!(
                            "已发现 {} 台接收设备\r\n选择可连接的设备，即可开始发送声音。",
                            self.peers.len()
                        ),
                        BLUE,
                    );
                }
            }
        }
        let Some(discovery) = &self.discovery else {
            return;
        };
        let snapshot = discovery.snapshot();
        let requests = discovery.take_requests();
        let peers: Vec<_> = snapshot
            .peers
            .into_iter()
            .filter(|p| p.identity.role == Role::Receiver)
            .collect();
        let signature = format!("{peers:?}");
        if signature != self.signature {
            unsafe {
                let list = self.control(LIST);
                let index = SendMessageW(list, LB_GETCURSEL, 0, 0);
                let selected = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.peers.get(i))
                    .map(|p| p.identity.id.clone());
                SendMessageW(list, WM_SETREDRAW, 0, 0);
                SendMessageW(list, LB_RESETCONTENT, 0, 0);
                for peer in &peers {
                    let label = format!(
                        "{}\t{} · {}",
                        peer.identity.name,
                        peer.address.ip(),
                        if peer.identity.ready {
                            "可连接"
                        } else {
                            "请先开始接收"
                        }
                    );
                    SendMessageW(list, LB_ADDSTRING, 0, wide(&label).as_ptr() as isize);
                }
                let chosen = selected
                    .and_then(|id| peers.iter().position(|p| p.identity.id == id))
                    .or_else(|| peers.iter().position(|p| p.identity.ready))
                    .unwrap_or(0);
                if !peers.is_empty() {
                    SendMessageW(list, LB_SETCURSEL, chosen, 0);
                }
                ShowWindow(
                    self.control(209),
                    if peers.is_empty() { SW_SHOW } else { SW_HIDE },
                );
                SendMessageW(list, WM_SETREDRAW, 1, 0);
                InvalidateRect(list, null(), 1);
            }
            self.signature = signature;
            self.peers = peers;
            self.sync_controls();
        }
        if self.session.is_none()
            && self.identity.ready
            && let Some(request) = requests.first()
        {
            self.begin(request.peer.audio_address(), &request.peer.identity.name);
        }
    }
    fn refresh(&mut self) {
        if let Some(d) = &self.discovery {
            match d.probe((std::net::Ipv4Addr::BROADCAST, lanmix_discovery::PORT).into()) {
                Ok(()) => {
                    self.refresh_until = Some(Instant::now() + Duration::from_millis(1600));
                    unsafe {
                        SetWindowTextW(self.control(REFRESH), wide("正在查找…").as_ptr());
                        EnableWindow(self.control(REFRESH), 0);
                    }
                    if self.session.is_none() {
                        self.status("正在查找附近设备…\r\n请确认手机已打开 LanMix，并与电脑连接同一局域网。", BLUE);
                    }
                }
                Err(e) => self.status(&format!("刷新失败：{e:#}"), RED),
            }
        } else {
            match Discovery::start(self.identity.clone()) {
                Ok(d) => {
                    self.discovery = Some(d);
                    self.refresh();
                }
                Err(e) => self.status(&format!("自动发现不可用：{e:#}。可按地址连接。"), RED),
            }
        }
    }
    fn command(&mut self, id: usize, notification: usize) {
        match id {
            LIST if notification == LBN_SELCHANGE as usize => self.sync_controls(),
            CONNECT | LIST if id == CONNECT || notification == LBN_DBLCLK as usize => {
                if self.session.is_some() {
                    return;
                }
                let index = unsafe { SendMessageW(self.control(LIST), LB_GETCURSEL, 0, 0) };
                if let Some(peer) = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.peers.get(i))
                    .cloned()
                {
                    if peer.identity.ready {
                        self.begin(peer.audio_address(), &peer.identity.name);
                    } else {
                        self.status("接收端尚未就绪\r\n请先在手机点击「开始接收」。", BLUE);
                    }
                }
            }
            ADDRESS_CONNECT => {
                let value = unsafe { window_text(self.control(MANUAL)) };
                let input = value.trim();
                let address = input.parse::<SocketAddr>().or_else(|_| {
                    input
                        .parse::<std::net::IpAddr>()
                        .map(|ip| SocketAddr::new(ip, 46210))
                });
                match address {
                    Ok(address) if !address.ip().is_unspecified() && address.port() != 0 => {
                        self.begin(address, input)
                    }
                    _ => {
                        self.status("地址格式不正确\r\n请输入接收端 IP 或 IP:端口，例如 192.168.1.184:46210。", RED);
                        unsafe {
                            SetFocus(self.control(MANUAL));
                            SendMessageW(self.control(MANUAL), EM_SETSEL, 0, -1);
                        }
                    }
                }
            }
            STOP => self.stop(),
            ALLOW => {
                self.preferences.allow =
                    unsafe { SendMessageW(self.control(ALLOW), BM_GETCHECK, 0, 0) }
                        == BST_CHECKED as isize;
                self.preferences.save();
                self.publish();
            }
            REFRESH => self.refresh(),
            RESTORE => self.restore(),
            RECONNECT => {
                if let Some(target) = self.preferences.last.clone() {
                    self.begin(target.address, &target.name);
                }
            }
            EXIT => unsafe {
                DestroyWindow(self.hwnd);
            },
            n if (PEER_MENU..PEER_MENU + 64).contains(&n) => {
                if let Some(peer) = self.peers.get(n - PEER_MENU).cloned()
                    && peer.identity.ready
                {
                    self.begin(peer.audio_address(), &peer.identity.name);
                }
            }
            _ => {}
        }
    }
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = self.icon;
        let tip = match &self.session {
            Some(s) if s.stopping => "LanMix · 正在断开".into(),
            Some(s) => format!(
                "LanMix · {} {}",
                if s.ready {
                    "正在发送 →"
                } else {
                    "正在连接"
                },
                s.target.name
            ),
            None => "LanMix · 待命 · 右键连接设备".into(),
        };
        copy_wide(&mut data.szTip, &tip);
        data
    }
    fn install_tray(&mut self) -> bool {
        let mut data = self.tray_data();
        self.tray = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
        if self.tray {
            unsafe {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
        self.tray
    }
    fn update_tray(&self) {
        if self.tray {
            unsafe {
                Shell_NotifyIconW(NIM_MODIFY, &self.tray_data());
            }
        }
    }
    fn balloon(&self, title: &str, message: &str) {
        if !self.tray {
            return;
        }
        let mut data = self.tray_data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, message);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    fn hide(&mut self) {
        if !self.tray && !self.install_tray() {
            self.status("托盘暂不可用，窗口已保留\r\n请稍后重试。", RED);
            self.restore();
            return;
        }
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        if !self.hidden_notice {
            self.hidden_notice = true;
            self.balloon(
                "LanMix 已在后台运行",
                "声音发送会继续。点击托盘图标打开窗口，右键可连接、断开或退出。",
            );
        }
    }
    fn restore(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_RESTORE);
            SetForegroundWindow(self.hwnd);
        }
    }
    unsafe fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        unsafe {
            let dc = item.hDC;
            let mut rect = item.rcItem;
            SetBkMode(dc, TRANSPARENT as i32);
            if item.CtlID as usize == LIST {
                if item.itemID == u32::MAX {
                    return;
                }
                let selected = item.itemState & ODS_SELECTED != 0;
                fill(
                    dc,
                    &rect,
                    if selected {
                        rgb(235, 241, 255)
                    } else {
                        rgb(255, 255, 255)
                    },
                );
                let len = SendMessageW(item.hwndItem, LB_GETTEXTLEN, item.itemID as usize, 0);
                if !(0..2048).contains(&len) {
                    return;
                }
                let mut buffer = vec![0u16; len as usize + 1];
                SendMessageW(
                    item.hwndItem,
                    LB_GETTEXT,
                    item.itemID as usize,
                    buffer.as_mut_ptr() as isize,
                );
                let text = String::from_utf16_lossy(&buffer[..len as usize]);
                let (name, detail) = text.split_once('\t').unwrap_or((&text, ""));
                if selected {
                    fill(
                        dc,
                        &RECT {
                            right: rect.left + self.px(3),
                            ..rect
                        },
                        BLUE,
                    );
                }
                rect.left += self.px(16);
                rect.right -= self.px(12);
                rect.top += self.px(7);
                rect.bottom = rect.top + self.px(24);
                draw_text(
                    dc,
                    self.font,
                    name,
                    &rect,
                    INK,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
                rect.top += self.px(26);
                rect.bottom += self.px(26);
                draw_text(
                    dc,
                    self.small_font,
                    detail,
                    &rect,
                    MUTED,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
                if item.itemState & ODS_FOCUS != 0 {
                    let mut focus = item.rcItem;
                    InflateRect(&mut focus, -2, -2);
                    DrawFocusRect(dc, &focus);
                }
                return;
            }
            let primary = item.CtlID as usize == CONNECT;
            let disabled = item.itemState & ODS_DISABLED != 0;
            let pressed = item.itemState & ODS_SELECTED != 0;
            let mut data = 0usize;
            GetWindowSubclass(item.hwndItem, Some(button_proc), 1, &mut data);
            let amount = if data == 0 {
                0.0
            } else {
                (*(data as *const ButtonFeedback)).amount
            };
            let color = if disabled {
                rgb(232, 236, 243)
            } else if pressed {
                if primary {
                    rgb(43, 76, 177)
                } else {
                    rgb(211, 223, 249)
                }
            } else if primary {
                blend(BLUE, rgb(81, 123, 238), amount)
            } else {
                blend(rgb(232, 238, 251), rgb(219, 229, 251), amount)
            };
            fill(dc, &rect, BG);
            if pressed {
                InflateRect(&mut rect, -self.px(1), -self.px(1));
                rect.top += self.px(1);
            }
            rounded(dc, &rect, self.px(11), color);
            draw_text(
                dc,
                self.font,
                &window_text(item.hwndItem),
                &rect,
                if disabled {
                    MUTED
                } else if primary {
                    rgb(255, 255, 255)
                } else {
                    BLUE
                },
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if item.itemState & ODS_FOCUS != 0 {
                InflateRect(&mut rect, -4, -4);
                DrawFocusRect(dc, &rect);
            }
        }
    }
    unsafe fn paint(&self) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(self.hwnd, &mut paint);
            rounded(
                dc,
                &self.rect(24, 96, 552, 102),
                self.px(16),
                rgb(255, 255, 255),
            );
            rounded(
                dc,
                &self.rect(24, 96, 5, 102),
                self.px(2),
                self.status_color,
            );
            fill(dc, &self.rect(24, 635, 552, 1), rgb(226, 232, 241));
            EndPaint(self.hwnd, &paint);
        }
    }
}
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        ((((a >> shift) & 255u32) as f32 * (1.0 - t) + ((b >> shift) & 255u32) as f32 * t).round()
            as u32)
            << shift
    };
    channel(0) | channel(8) | channel(16)
}
unsafe fn window_text(hwnd: HWND) -> String {
    unsafe {
        let mut data = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let len = GetWindowTextW(hwnd, data.as_mut_ptr(), data.len() as i32);
        String::from_utf16_lossy(&data[..len.max(0) as usize])
    }
}
unsafe fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}
unsafe fn rounded(dc: HDC, rect: &RECT, radius: i32, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let old = SelectObject(dc, brush);
        let pen = SelectObject(dc, GetStockObject(NULL_PEN));
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, pen);
        SelectObject(dc, old);
        DeleteObject(brush);
    }
}
unsafe fn draw_text(dc: HDC, font: HFONT, text: &str, rect: &RECT, color: u32, flags: u32) {
    unsafe {
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut rect = *rect;
        DrawTextW(dc, wide(text).as_ptr(), -1, &mut rect, flags | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}
unsafe fn brand_icon() -> HICON {
    // Original waveform mark, rendered in Rust into BGRA.
    let mut pixels = [0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let inside = (x as f32 - 15.5).powi(2) + (y as f32 - 15.5).powi(2) < 15.0f32.powi(2);
            if inside {
                pixels[i..i + 4].copy_from_slice(&[222, 103, 62, 255]);
            }
            if inside
                && ((7..10).contains(&x) && (13..19).contains(&y)
                    || (12..15).contains(&x) && (8..24).contains(&y)
                    || (17..20).contains(&x) && (5..27).contains(&y)
                    || (22..25).contains(&x) && (11..21).contains(&y))
            {
                pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    unsafe {
        CreateIcon(
            GetModuleHandleW(null()),
            32,
            32,
            1,
            32,
            [0u8; 128].as_ptr(),
            pixels.as_ptr(),
        )
    }
}

unsafe fn tray_menu(ptr: *mut Window) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            (*ptr).restore();
            return;
        }
        let hwnd = (*ptr).hwnd;
        let idle = (*ptr).session.is_none();
        let disabled = if idle {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        AppendMenuW(menu, MF_STRING, RESTORE, wide("打开 LanMix").as_ptr());
        SetMenuDefaultItem(menu, RESTORE as u32, 0);
        let description = (*ptr).session.as_ref().map_or("待命 · 未连接".into(), |s| {
            format!(
                "{} {}",
                if s.stopping {
                    "正在断开"
                } else if s.ready {
                    "正在发送 →"
                } else {
                    "正在连接"
                },
                s.target.name
            )
        });
        AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED,
            0,
            wide(&description.replace('&', "&&")).as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        if (*ptr).peers.is_empty() {
            AppendMenuW(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                wide("未发现接收设备").as_ptr(),
            );
        }
        // Freeze the command-to-peer mapping while the nested popup loop runs.
        let peers = (*ptr).peers.clone();
        for (i, peer) in peers.iter().enumerate() {
            AppendMenuW(
                menu,
                disabled | if peer.identity.ready { 0 } else { MF_GRAYED },
                PEER_MENU + i,
                wide(&format!(
                    "连接 {}{}",
                    peer.identity.name.replace('&', "&&"),
                    if peer.identity.ready {
                        ""
                    } else {
                        "（未就绪）"
                    }
                ))
                .as_ptr(),
            );
        }
        if let Some(target) = &(*ptr).preferences.last {
            AppendMenuW(
                menu,
                disabled,
                RECONNECT,
                wide(&format!("重新连接 {}", target.name.replace('&', "&&"))).as_ptr(),
            );
        }
        AppendMenuW(
            menu,
            MF_STRING
                | if (*ptr).session.as_ref().is_some_and(|s| !s.stopping) {
                    0
                } else {
                    MF_GRAYED
                },
            STOP,
            wide("断开连接").as_ptr(),
        );
        AppendMenuW(menu, MF_STRING, REFRESH, wide("刷新设备").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, EXIT, wide("退出 LanMix").as_ptr());
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        // No reference to Window is held across TrackPopupMenu's nested dispatch.
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            hwnd,
            null(),
        ) as usize;
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        if (PEER_MENU..PEER_MENU + peers.len()).contains(&selected) {
            let chosen = &peers[selected - PEER_MENU];
            if let Some(peer) = (*ptr)
                .peers
                .iter()
                .find(|p| p.identity.id == chosen.identity.id && p.identity.ready)
                .cloned()
            {
                (*ptr).begin(peer.audio_address(), &peer.identity.name);
            }
        } else if selected != 0 {
            PostMessageW(hwnd, WM_COMMAND, selected, 0);
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(l as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Window;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        if msg == WM_SYSCOMMAND && w & 0xfff0 == SYSTEM_EXIT {
            DestroyWindow(hwnd);
            return 0;
        }
        if msg == WM_CTLCOLORSTATIC || msg == WM_CTLCOLORBTN || msg == WM_CTLCOLORLISTBOX {
            let id = GetDlgCtrlID(l as HWND);
            let dc = w as HDC;
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(
                dc,
                if id == 200 {
                    (*ptr).status_color
                } else if id == 201 || id == 208 {
                    INK
                } else {
                    MUTED
                },
            );
            let card = id == 200 || id == 208 || msg == WM_CTLCOLORLISTBOX || id == 209;
            SetBkColor(dc, if card { rgb(255, 255, 255) } else { BG });
            return if card {
                (*ptr).white
            } else {
                (*ptr).background
            } as LRESULT;
        }
        if msg == WM_DRAWITEM {
            (*ptr).draw_item(&*(l as *const DRAWITEMSTRUCT));
            return 1;
        }
        if msg == WM_MEASUREITEM {
            (*(l as *mut MEASUREITEMSTRUCT)).itemHeight = (*ptr).px(62) as u32;
            return 1;
        }
        if msg == WM_PAINT {
            (*ptr).paint();
            return 0;
        }
        if msg == (*ptr).taskbar_created && msg != 0 {
            (*ptr).tray = false;
            if !(*ptr).install_tray() {
                (*ptr).restore();
            }
            return 0;
        }
        if msg == WM_COMMAND
            || msg == WM_TIMER
            || msg == WM_CLOSE
            || msg == WM_SIZE
            || msg == TRAY
            || msg == SHOW
        {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match msg {
                WM_COMMAND => (*ptr).command(w & 0xffff, (w >> 16) & 0xffff),
                WM_TIMER => (*ptr).tick(),
                WM_CLOSE => (*ptr).hide(),
                WM_SIZE if w == SIZE_MINIMIZED as usize => (*ptr).hide(),
                SHOW => (*ptr).restore(),
                TRAY => match l as u32 & 0xffff {
                    WM_CONTEXTMENU | WM_RBUTTONUP => tray_menu(ptr),
                    NIN_SELECT | NIN_KEYSELECT | WM_LBUTTONUP => (*ptr).restore(),
                    _ => {}
                },
                _ => {}
            }));
            if outcome.is_err() {
                DestroyWindow(hwnd);
                PostQuitMessage(1);
            }
            if msg != WM_SIZE {
                return 0;
            }
        }
        if msg == WM_DESTROY {
            KillTimer(hwnd, 1);
            if (*ptr).tray {
                Shell_NotifyIconW(NIM_DELETE, &(*ptr).tray_data());
                (*ptr).tray = false;
            }
            PostQuitMessage(0);
            return 0;
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
pub fn run() -> Result<()> {
    unsafe {
        let class_name = wide(CLASS);
        let existing = FindWindowW(class_name.as_ptr(), null());
        if !existing.is_null() {
            PostMessageW(existing, SHOW, 0, 0);
            return Ok(());
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let system_dpi = GetDpiForSystem() as f32 / 96.0;
        let mut work = RECT::default();
        let available =
            SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) != 0;
        let dpi = if available {
            system_dpi.min(((work.bottom - work.top - 80) as f32 / 690.0).max(0.75))
        } else {
            system_dpi
        };
        let make_font = |size: f32, weight: u32| {
            CreateFontW(
                -(size * dpi).round() as i32,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Microsoft YaHei UI").as_ptr(),
            )
        };
        let font = make_font(15.0, FW_NORMAL);
        let small_font = make_font(12.0, FW_NORMAL);
        let title_font = make_font(30.0, FW_SEMIBOLD);
        let background = CreateSolidBrush(BG);
        let white = CreateSolidBrush(rgb(255, 255, 255));
        let icon = brand_icon();
        let identity = sender_identity(true)?;
        let discovery = Discovery::start(identity.clone());
        let error = discovery
            .as_ref()
            .err()
            .map(|e| format!("自动发现不可用：{e:#}。仍可按地址连接。"));
        let mut state = Box::new(Window {
            hwnd: null_mut(),
            identity,
            discovery: discovery.ok(),
            peers: Vec::new(),
            signature: String::new(),
            session: None,
            controls: Vec::new(),
            feedback: Vec::new(),
            font,
            small_font,
            title_font,
            background,
            white,
            status_color: BLUE,
            dpi,
            outputs: Vec::new(),
            preferences: Preferences::load(),
            icon,
            tray: false,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            refresh_until: None,
            hidden_notice: false,
        });
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: background,
            hIcon: icon,
            ..std::mem::zeroed()
        };
        ensure!(RegisterClassW(&class) != 0, "register sender window");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let mut rect = state.rect(0, 0, 600, 690);
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("LanMix · 发送声音").as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut Window).cast(),
        );
        ensure!(!hwnd.is_null(), "create sender window");
        state.hwnd = hwnd;
        let system_menu = GetSystemMenu(hwnd, 0);
        AppendMenuW(system_menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            system_menu,
            MF_STRING,
            SYSTEM_EXIT,
            wide("退出 LanMix").as_ptr(),
        );
        if let Err(e) = state.init() {
            DestroyWindow(hwnd);
            return Err(e);
        }
        if let Some(error) = error {
            state.status(&error, RED);
        }
        state.install_tray();
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut message: MSG = std::mem::zeroed();
        let mut result;
        loop {
            result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if message.message == WM_KEYDOWN && message.wParam == VK_RETURN as usize {
                let focus = GetFocus();
                if focus == state.control(MANUAL) || focus == state.control(LIST) {
                    state.command(
                        if focus == state.control(MANUAL) {
                            ADDRESS_CONNECT
                        } else {
                            CONNECT
                        },
                        0,
                    );
                    continue;
                }
            }
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Some(session) = &state.session {
            announce(
                &state.discovery,
                session.target.address,
                1,
                SessionState::Disconnected,
            );
        }
        state.session.take();
        state.discovery.take();
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        DeleteObject(font);
        DeleteObject(small_font);
        DeleteObject(title_font);
        DeleteObject(background);
        DeleteObject(white);
        DestroyIcon(icon);
        ensure!(
            result != -1 && message.wParam == 0,
            "native sender window failed"
        );
        Ok(())
    }
}
