//! System media keys are delivered outside the capture callback and PCM sender.
use super::*;
use lanmix_discovery::{Discovery, MediaAction};

pub fn dispatch_pending(discovery: &Discovery, destination: Option<(SocketAddr, u32)>) {
    for request in discovery.take_media_requests() {
        if destination != Some((request.peer.audio_address(), request.stream_id)) {
            continue;
        }
        if let Err(error) = dispatch(request.action) {
            eprintln!("Remote media control failed: {error:#}");
        }
    }
}

#[cfg(windows)]
fn keyboard_inputs(
    action: MediaAction,
) -> [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; 2] {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let key = match action {
        MediaAction::Previous => VK_MEDIA_PREV_TRACK,
        MediaAction::PlayPause => VK_MEDIA_PLAY_PAUSE,
        MediaAction::Next => VK_MEDIA_NEXT_TRACK,
    };
    [
        KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
    ]
    .map(|flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}

fn dispatch(action: MediaAction) -> Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let inputs = keyboard_inputs(action);
        let count = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        ensure!(
            count == 2,
            "Windows accepted {count}/2 media key events; an elevated player may block input"
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        bail!("system media controls are currently Windows-only")
    }
}

pub struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        discovery: Arc<Discovery>,
        destination: SocketAddr,
        stream_id: u32,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let handle = thread::Builder::new()
            .name("lanmix-media".into())
            .spawn(move || {
                while !halt.load(Ordering::Acquire) {
                    dispatch_pending(&discovery, Some((destination, stream_id)));
                    thread::sleep(Duration::from_millis(30));
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
