//! Small Win32 UI in Rust; no WebView, renderer, async runtime or child process.
use super::*;
use lanmix_discovery::{Discovery, Identity, Peer, Role};
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::mpsc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::BST_CHECKED, HiDpi::*, Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const LIST: usize = 101;
const CONNECT: usize = 102;
const MANUAL: usize = 103;
const STOP: usize = 104;
const ALLOW: usize = 105;
const OUTPUT: usize = 106;
const REFRESH: usize = 107;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Session {
    running: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    result: mpsc::Receiver<std::result::Result<(), String>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Window {
    identity: Identity,
    discovery: Option<Discovery>,
    peers: Vec<Peer>,
    signature: String,
    session: Option<Session>,
    controls: Vec<(usize, HWND)>,
    font: HFONT,
    title_font: HFONT,
    status: String,
    dpi: f32,
    outputs: Vec<usize>,
}
impl Window {
    fn control(&self, id: usize) -> HWND {
        self.controls
            .iter()
            .find(|(key, _)| *key == id)
            .map_or(null_mut(), |(_, handle)| *handle)
    }
    fn status(&mut self, message: &str) {
        self.status = message.into();
        // SAFETY: controls are owned by the live parent HWND; UTF-16 lasts for the call.
        unsafe {
            SetWindowTextW(self.control(200), wide(message).as_ptr());
        }
    }
    unsafe fn add(
        &mut self,
        parent: HWND,
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
                parent,
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
                } else {
                    self.font
                } as usize,
                1,
            );
            self.controls.push((id, handle));
            Ok(())
        }
    }
    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.dpi).round() as i32
    }
    unsafe fn init(&mut self, parent: HWND) -> Result<()> {
        unsafe {
            self.add(parent, "STATIC", "LanMix", 201, (24, 20, 300, 40), 0)?;
            self.add(
                parent,
                "STATIC",
                "把这台电脑的声音，送到你想听的地方。",
                202,
                (24, 65, 540, 25),
                0,
            )?;
            self.add(parent, "STATIC", "附近的接收端", 203, (24, 108, 320, 24), 0)?;
            self.add(
                parent,
                "LISTBOX",
                "",
                LIST,
                (24, 137, 544, 140),
                WS_BORDER | WS_VSCROLL | LBS_NOTIFY as u32 | WS_TABSTOP,
            )?;
            self.add(
                parent,
                "BUTTON",
                "连接选中设备",
                CONNECT,
                (24, 289, 354, 38),
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            )?;
            self.add(
                parent,
                "BUTTON",
                "刷新",
                REFRESH,
                (390, 289, 178, 38),
                WS_TABSTOP,
            )?;
            self.add(
                parent,
                "STATIC",
                "或输入接收端 IP:端口",
                204,
                (24, 346, 544, 22),
                0,
            )?;
            self.add(
                parent,
                "EDIT",
                "",
                MANUAL,
                (24, 375, 354, 34),
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            )?;
            self.add(
                parent,
                "BUTTON",
                "按地址连接",
                108,
                (390, 373, 178, 38),
                WS_TABSTOP,
            )?;
            self.add(parent, "STATIC", "采集输出设备", 205, (24, 426, 135, 28), 0)?;
            self.add(
                parent,
                "COMBOBOX",
                "",
                OUTPUT,
                (166, 421, 402, 250),
                WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
            )?;
            SendMessageW(
                self.control(OUTPUT),
                CB_ADDSTRING,
                0,
                wide("系统默认输出（回环采集）").as_ptr() as isize,
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
                Err(error) => self.status(&format!("音频设备查询失败：{error:#}")),
            }
            SendMessageW(self.control(OUTPUT), CB_SETCURSEL, 0, 0);
            self.add(
                parent,
                "BUTTON",
                "允许接收端发起连接（空闲时）",
                ALLOW,
                (24, 470, 544, 30),
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            )?;
            SendMessageW(self.control(ALLOW), BM_SETCHECK, BST_CHECKED as usize, 0);
            self.add(
                parent,
                "STATIC",
                "正在发现局域网接收端…",
                200,
                (24, 517, 544, 62),
                0,
            )?;
            self.add(
                parent,
                "BUTTON",
                "停止发送",
                STOP,
                (24, 588, 544, 38),
                WS_TABSTOP,
            )?;
            self.add(
                parent,
                "STATIC",
                "原始 PCM · 独立后台线程 · 关闭窗口即停止发送",
                206,
                (24, 643, 544, 24),
                0,
            )?;
            SetTimer(parent, 1, 400, None);
            Ok(())
        }
    }
    fn begin(&mut self, address: SocketAddr, name: &str) {
        if self.session.is_some() {
            self.status("请先停止当前发送，再切换接收设备。");
            return;
        }
        // SAFETY: read selection from an existing combo box; indexes are checked below.
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
        let (tx, result) = mpsc::channel();
        match thread::Builder::new()
            .name("lanmix-capture".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(|| send(options, flag))
                    .map_err(|_| "音频线程异常退出".to_string())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(outcome);
            }) {
            Ok(worker) => {
                self.session = Some(Session {
                    running,
                    worker: Some(worker),
                    result,
                });
                self.status(&format!(
                    "正在发送 → {name}\r\n{address} · 电脑输出保持原样"
                ));
            }
            Err(e) => self.status(&format!("无法启动采集：{e}")),
        }
        self.publish();
    }
    fn publish(&mut self) {
        // SAFETY: only called after the checkbox is created, on the window thread.
        let allow =
            unsafe { SendMessageW(self.control(ALLOW), BM_GETCHECK, 0, 0) } == BST_CHECKED as isize;
        self.identity.ready = allow && self.session.is_none();
        if let Some(discovery) = &self.discovery {
            let _ = discovery.update(self.identity.clone());
        }
        unsafe {
            EnableWindow(self.control(OUTPUT), i32::from(self.session.is_none()));
        }
    }
    fn tick(&mut self) {
        if let Some(outcome) = self.session.as_ref().and_then(|s| s.result.try_recv().ok()) {
            self.session.take();
            self.status(&match outcome {
                Ok(()) => "已停止发送".into(),
                Err(e) => format!("发送失败：{e}"),
            });
            self.publish();
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
                        "{}  ·  {}  ·  {}",
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
                    .unwrap_or(0);
                if !peers.is_empty() {
                    SendMessageW(list, LB_SETCURSEL, chosen, 0);
                }
                SendMessageW(list, WM_SETREDRAW, 1, 0);
                InvalidateRect(list, null(), 1);
            }
            self.signature = signature;
            self.peers = peers;
        }
        if self.session.is_none() {
            let allow = unsafe { SendMessageW(self.control(ALLOW), BM_GETCHECK, 0, 0) }
                == BST_CHECKED as isize;
            if allow && let Some(request) = requests.first() {
                self.begin(request.peer.audio_address(), &request.peer.identity.name);
            }
        }
    }
    fn command(&mut self, id: usize, notification: usize) {
        match id {
            CONNECT | LIST if id == CONNECT || notification == LBN_DBLCLK as usize => {
                let index = unsafe { SendMessageW(self.control(LIST), LB_GETCURSEL, 0, 0) };
                if let Some(peer) = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.peers.get(i))
                    .cloned()
                {
                    if peer.identity.ready {
                        self.begin(peer.audio_address(), &peer.identity.name);
                    } else {
                        self.status("请先在接收设备点击「开始接收」。");
                    }
                } else {
                    self.status("还没有发现接收端，请确认两端在同一局域网并已启动 LanMix。");
                }
            }
            108 => {
                let mut value = [0u16; 256];
                let len = unsafe {
                    GetWindowTextW(self.control(MANUAL), value.as_mut_ptr(), value.len() as i32)
                };
                let value = String::from_utf16_lossy(&value[..len as usize]);
                match value.trim().parse() {
                    Ok(address) => self.begin(address, value.trim()),
                    Err(_) => self.status("请输入接收端 IP:端口，例如 192.168.1.184:46210"),
                }
            }
            STOP => {
                self.session.take();
                self.status("已停止发送，等待下一次连接。");
                self.publish();
            }
            ALLOW => self.publish(),
            REFRESH => {
                // A real re-probe, rather than an empty visual refresh.
                if let Some(d) = &self.discovery {
                    let _ = d.probe((std::net::Ipv4Addr::BROADCAST, lanmix_discovery::PORT).into());
                }
                self.status("正在刷新，设备通常会在几秒内出现。");
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: caller is Win32. GWLP_USERDATA points to the Box kept alive in run().
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        if message == WM_CTLCOLORSTATIC {
            let dc = w as HDC;
            SetBkMode(dc, TRANSPARENT as i32);
            let id = GetDlgCtrlID(l as HWND);
            SetTextColor(
                dc,
                if id == 201 {
                    0x00DE673E
                } else if id == 200 {
                    0x0037261D
                } else {
                    0x00927E73
                },
            );
            return GetSysColorBrush(COLOR_WINDOW) as LRESULT;
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Window;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, message, w, l);
        }
        // Do not call handlers while building controls: CreateWindow can re-enter the WndProc.
        if message == WM_COMMAND || message == WM_TIMER || message == WM_CLOSE {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let state = &mut *ptr;
                match message {
                    WM_COMMAND => state.command(w & 0xffff, (w >> 16) & 0xffff),
                    WM_TIMER => state.tick(),
                    WM_CLOSE => {
                        state.session.take();
                        KillTimer(hwnd, 1);
                    }
                    _ => {}
                }
            }));
            if outcome.is_err() {
                PostQuitMessage(1);
                return 0;
            }
            if message == WM_CLOSE {
                DestroyWindow(hwnd);
            }
            return 0;
        }
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}

pub fn run() -> Result<()> {
    // CLI commands keep their console. Only the default GUI detaches a console on launch.
    let identity = sender_identity(true)?;
    let discovery = Discovery::start(identity.clone());
    let error = discovery
        .as_ref()
        .err()
        .map(|e| format!("自动发现不可用：{e:#}。仍可手动连接。"));
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let dpi = GetDpiForSystem() as f32 / 96.0;
        let font = CreateFontW(
            -(16.0 * dpi) as i32,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Microsoft YaHei UI").as_ptr(),
        );
        let title_font = CreateFontW(
            -(30.0 * dpi) as i32,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let mut state = Box::new(Window {
            identity,
            discovery: discovery.ok(),
            peers: Vec::new(),
            signature: String::new(),
            session: None,
            controls: Vec::new(),
            font,
            title_font,
            status: String::new(),
            dpi,
            outputs: Vec::new(),
        });
        let instance = GetModuleHandleW(null());
        let class_name = wide("LanMixSenderWindow");
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            ..std::mem::zeroed()
        };
        ensure!(RegisterClassW(&class) != 0, "register sender window");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("LanMix · 发送声音").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            state.px(610),
            state.px(720),
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut Window).cast(),
        );
        ensure!(!hwnd.is_null(), "create sender window");
        if let Err(e) = state.init(hwnd) {
            DestroyWindow(hwnd);
            return Err(e);
        }
        if let Some(error) = error {
            state.status(&error);
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut message: MSG = std::mem::zeroed();
        let mut result;
        loop {
            result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        state.session.take();
        state.discovery.take();
        DeleteObject(font);
        DeleteObject(title_font);
        ensure!(
            result != -1 && message.wParam == 0,
            "native sender window failed"
        );
    }
    Ok(())
}
