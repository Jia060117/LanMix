//! Portable JNI bridge. Host cargo check validates the same exports as Android.
//! Kotlin owns AudioTrack; nativeFill only renders and copies preallocated PCM.
use anyhow::{Context, Result, anyhow, ensure};
use jni::{
    JNIEnv,
    objects::{JFloatArray, JObject, JString},
    sys::{jboolean, jfloat, jint, jstring},
};
use lanmix_core::Mixer;
use lanmix_network::{Receiver, ReceiverConfig};
use std::{
    cell::RefCell,
    net::{Ipv4Addr, SocketAddr},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_FRAMES: usize = 4096;
static DISCOVERY: Mutex<Option<lanmix_discovery::Discovery>> = Mutex::new(None);
static RECEIVER: Mutex<Option<Receiver>> = Mutex::new(None);
// First fill consumes this startup handoff. UI methods never touch it;
// subsequent fills use a worker-owned mixer without any global render lock.
static PENDING_RENDER: Mutex<Option<LocalRender>> = Mutex::new(None);
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_EPOCH: AtomicU64 = AtomicU64::new(0);
struct LocalRender {
    epoch: u64,
    render: Render,
}
thread_local! {
    static LOCAL_RENDER: RefCell<Option<LocalRender>> = const { RefCell::new(None) };
}
struct Render {
    mixer: Box<Mixer>,
    frames: Box<[[f32; 2]; MAX_FRAMES]>,
    interleaved: Box<[f32; MAX_FRAMES * 2]>,
}

fn start(port: jint, buffer_ms: jint, sample_rate: jint) -> Result<()> {
    ensure!((1..=65535).contains(&port), "port must be 1..65535");
    ensure!((20..=200).contains(&buffer_ms), "buffer must be 20..200 ms");
    ensure!(
        (8000..=192000).contains(&sample_rate),
        "sample rate must be 8000..192000 Hz"
    );
    // Only start/stop touch both lifecycle and startup handoff locks.
    let mut receiver = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    ensure!(
        receiver.is_none(),
        "receiver already running; stop before restarting"
    );
    // JNI callers commonly have only a 1 MiB stack. Construct and box the
    // relatively large mixer on an explicit initialization stack, including in
    // unoptimized host builds. Only small heap owners cross the JNI stack.
    let (next, render) = std::thread::Builder::new()
        .name("lanmix-init".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<(Receiver, Render)> {
            let (next, mixer) = Receiver::bind_with_muted_sources(
                ReceiverConfig {
                    bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port as u16)),
                    buffer_ms: buffer_ms as u32,
                },
                sample_rate as u32,
            )?;
            Ok((
                next,
                Render {
                    mixer: Box::new(mixer),
                    frames: Box::new([[0.0; 2]; MAX_FRAMES]),
                    interleaved: Box::new([0.0; MAX_FRAMES * 2]),
                },
            ))
        })?
        .join()
        .map_err(|_| anyhow!("native mixer initialization panicked"))??;
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    *PENDING_RENDER
        .lock()
        .map_err(|_| anyhow!("startup handoff lock poisoned"))? =
        Some(LocalRender { epoch, render });
    *receiver = Some(next);
    ACTIVE_EPOCH.store(epoch, Ordering::Release);
    Ok(())
}

fn stop() -> Result<()> {
    let mut receiver = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    ACTIVE_EPOCH.store(0, Ordering::Release);
    PENDING_RENDER
        .lock()
        .map_err(|_| anyhow!("startup handoff lock poisoned"))?
        .take();
    LOCAL_RENDER.with(|state| state.borrow_mut().take());
    // Receiver::drop joins the network worker before another start can bind.
    drop(receiver.take());
    Ok(())
}

fn fill(length: usize, copy: impl FnOnce(&[f32]) -> Result<()>) -> Result<()> {
    ensure!(
        length > 0 && length.is_multiple_of(2) && length <= MAX_FRAMES * 2,
        "PCM array must have an even length in 2..8192 (stereo interleaved)"
    );
    let epoch = ACTIVE_EPOCH.load(Ordering::Acquire);
    ensure!(epoch != 0, "receiver is stopped");
    LOCAL_RENDER.with(|state| {
        let mut local = state.borrow_mut();
        if local.as_ref().is_none_or(|state| state.epoch != epoch) {
            local.take();
            let mut pending = PENDING_RENDER
                .lock()
                .map_err(|_| anyhow!("startup handoff lock poisoned"))?;
            ensure!(
                pending.as_ref().is_some_and(|state| state.epoch == epoch),
                "nativeFill must remain on one audio worker until restart"
            );
            *local = pending.take();
        }
        let render = &mut local.as_mut().context("render startup failed")?.render;
        render.mixer.render(&mut render.frames[..length / 2]);
        for (frame, output) in render.frames[..length / 2]
            .iter()
            .zip(render.interleaved[..length].as_chunks_mut::<2>().0)
        {
            output.copy_from_slice(frame);
        }
        // No global lock, allocation, pinning, or socket calls after handoff.
        copy(&render.interleaved[..length])
    })
}

fn tracks() -> Result<String> {
    let state = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    let Some(receiver) = state.as_ref() else {
        return Ok(r#"{"tracks":[],"stats":{},"meters":[],"running":false}"#.into());
    };
    let meters: Vec<_> = receiver.controls.iter().enumerate().map(|(slot, c)| {
        serde_json::json!({"slot": slot, "peak": c.peak(), "underruns": c.underruns(),
            "buffered_ms": c.buffered_frames() as f64 * 1000.0 / c.sample_rate() as f64,
            "target_buffer_ms": c.target_buffer_ms(), "primed": c.primed(), "drift_ppm": c.drift_ppm()})
    }).collect();
    Ok(serde_json::json!({"running": true, "tracks": receiver.snapshot(), "stats": receiver.stats(), "meters": meters,
        "master": {"peak": receiver.master.peak(), "clipped_frames": receiver.master.clipped_frames()}}).to_string())
}

// Control-plane topology only. Frequent source admission must not serialize all
// meters/loss history or wait for the UI's much slower diagnostic refresh.
fn sources() -> Result<String> {
    let state = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    let snapshots = state.as_ref().map(Receiver::snapshot).unwrap_or_default();
    let sources: Vec<_> = snapshots
        .iter()
        .map(|source| {
            serde_json::json!({"slot": source.slot, "source": source.source,
                "stream_id": source.stream_id, "sample_rate": source.sample_rate,
                "online": source.online})
        })
        .collect();
    Ok(serde_json::to_string(&sources)?)
}

fn read_bands(env: &JNIEnv<'_>, array: &JFloatArray<'_>) -> Result<[f32; 10]> {
    ensure!(
        env.get_array_length(array)? == 10,
        "EQ requires exactly ten bands"
    );
    let mut result = [0.0; 10];
    env.get_float_array_region(array, 0, &mut result)?;
    ensure!(
        result.iter().all(|v| v.is_finite()),
        "EQ bands must be finite"
    );
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn set_track(
    index: jint,
    gain: f32,
    muted: bool,
    solo: bool,
    enabled: bool,
    preamp: f32,
    bands: [f32; 10],
) -> Result<()> {
    ensure!((0..16).contains(&index), "track index must be 0..15");
    ensure!(
        gain.is_finite() && preamp.is_finite(),
        "gain and preamp must be finite"
    );
    let state = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    let receiver = state.as_ref().context("receiver is stopped")?;
    let control = receiver
        .controls
        .get(index as usize)
        .context("track slot unavailable")?;
    control.set_gain_db(gain);
    control.set_solo(solo);
    control.set_eq_enabled(enabled);
    control.set_preamp_db(preamp);
    for (i, db) in bands.into_iter().enumerate() {
        control.set_eq_band(i, db);
    }
    // Open a new source's mute gate only after its complete tone/solo settings.
    control.set_muted(muted);
    Ok(())
}

fn set_master(gain: f32, enabled: bool, preamp: f32, bands: [f32; 10]) -> Result<()> {
    ensure!(
        gain.is_finite() && preamp.is_finite(),
        "gain and preamp must be finite"
    );
    let state = RECEIVER
        .lock()
        .map_err(|_| anyhow!("receiver lock poisoned"))?;
    let receiver = state.as_ref().context("receiver is stopped")?;
    receiver.master.set_gain_db(gain);
    receiver.master.set_eq_enabled(enabled);
    receiver.master.set_preamp_db(preamp);
    for (i, db) in bands.into_iter().enumerate() {
        receiver.master.set_eq_band(i, db);
    }
    Ok(())
}

/// No unwinding across JNI. Preserve any exception already pending in the JVM.
fn boundary<T: Default>(
    env: &mut JNIEnv<'_>,
    action: impl FnOnce(&mut JNIEnv<'_>) -> Result<T>,
) -> T {
    match catch_unwind(AssertUnwindSafe(|| action(env))) {
        Ok(Ok(value)) => value,
        error => {
            let message = match error {
                Ok(Err(error)) => format!("{error:#}"),
                Err(_) => "native receiver panicked; restart application".into(),
                _ => unreachable!(),
            };
            if !env.exception_check().unwrap_or(true) {
                let _ = env.throw_new("java/lang/IllegalStateException", message);
            }
            T::default()
        }
    }
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeStart(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    port: jint,
    buffer_ms: jint,
    rate: jint,
) {
    boundary(&mut env, |_| start(port, buffer_ms, rate));
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeStop(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) {
    boundary(&mut env, |_| stop());
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeFill(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    array: JFloatArray<'_>,
) {
    boundary(&mut env, |env| {
        fill(env.get_array_length(&array)? as usize, |pcm| {
            env.set_float_array_region(&array, 0, pcm)?;
            Ok(())
        })
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeTracks(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) -> jstring {
    boundary(&mut env, |env| Ok(env.new_string(tracks()?)?.into_raw()))
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeSources(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) -> jstring {
    boundary(&mut env, |env| Ok(env.new_string(sources()?)?.into_raw()))
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeSetTrack(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    index: jint,
    gain: jfloat,
    muted: jboolean,
    solo: jboolean,
    enabled: jboolean,
    preamp: jfloat,
    array: JFloatArray<'_>,
) {
    boundary(&mut env, |env| {
        set_track(
            index,
            gain,
            muted != 0,
            solo != 0,
            enabled != 0,
            preamp,
            read_bands(env, &array)?,
        )
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeSetMaster(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    gain: jfloat,
    enabled: jboolean,
    preamp: jfloat,
    array: JFloatArray<'_>,
) {
    boundary(&mut env, |env| {
        set_master(gain, enabled != 0, preamp, read_bands(env, &array)?)
    });
}

/// Discovery has its own worker/lock and never shares the PCM lifecycle mutex.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoveryUpdate(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    json: JString<'_>,
) {
    boundary(&mut env, |env| {
        let json: String = env.get_string(&json)?.into();
        let identity: lanmix_discovery::Identity = serde_json::from_str(&json)?;
        let mut discovery = DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?;
        if let Some(active) = discovery.as_ref() {
            active.update(identity)?;
        } else {
            *discovery = Some(lanmix_discovery::Discovery::start(identity)?);
        }
        Ok(())
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoverySnapshot(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) -> jstring {
    boundary(&mut env, |env| {
        let discovery = DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?;
        let snapshot = discovery.as_ref().map(|d| d.snapshot()).unwrap_or_default();
        Ok(env
            .new_string(serde_json::to_string(&snapshot)?)?
            .into_raw())
    })
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoveryConnect(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    id: JString<'_>,
) {
    boundary(&mut env, |env| {
        let id: String = env.get_string(&id)?.into();
        DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?
            .as_ref()
            .context("discovery is stopped")?
            .connect(&id)
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoveryMedia(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    id: JString<'_>,
    revision: jni::sys::jlong,
    action: JString<'_>,
) {
    boundary(&mut env, |env| {
        ensure!(revision > 0, "invalid media session revision");
        let id: String = env.get_string(&id)?.into();
        let action: String = env.get_string(&action)?.into();
        let action: lanmix_discovery::MediaAction =
            serde_json::from_value(serde_json::Value::String(action))?;
        DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?
            .as_ref()
            .context("discovery is stopped")?
            .media(&id, revision as u64, action)
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoveryProbe(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    address: JString<'_>,
) {
    boundary(&mut env, |env| {
        let address: String = env.get_string(&address)?.into();
        DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?
            .as_ref()
            .context("discovery is stopped")?
            .probe(address.parse()?)
    });
}
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_lanmix_receiver_NativeBridge_nativeDiscoveryStop(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) {
    boundary(&mut env, |_| {
        DISCOVERY
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?
            .take();
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_inputs_and_stopped_lifecycle() {
        assert!(start(0, 60, 48000).is_err());
        assert!(start(46210, 19, 48000).is_err());
        assert!(start(46210, 60, 192001).is_err());
        for len in [0, 1, 3, MAX_FRAMES * 2 + 2] {
            assert!(fill(len, |_| panic!("invalid copy")).is_err());
        }
        assert!(set_track(16, 0.0, false, false, false, 0.0, [0.0; 10]).is_err());
        assert!(set_master(f32::NAN, false, 0.0, [0.0; 10]).is_err());
        stop().unwrap();
        let state: serde_json::Value = serde_json::from_str(&tracks().unwrap()).unwrap();
        assert_eq!(state["running"], false);
        assert_eq!(sources().unwrap(), "[]");
        assert!(fill(2048, |_| Ok(())).is_err());
    }
}
