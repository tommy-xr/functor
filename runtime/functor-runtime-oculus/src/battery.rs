//! `/state.battery`: Android `BatteryManager` read over JNI on a background
//! thread, at most once per second. `current_now` in sysfs is
//! permission-denied over adb on Quest, but the app can read it in-process.

use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use functor_runtime_common::debug_protocol::{AndroidBatteryRaw, BatteryState};
use jni::objects::JObject;
use jni::refs::Global;
use jni::{jni_sig, jni_str, Env, JValue, JavaVM};

/// The VM and the Application context, which (unlike the Activity) lives as
/// long as the process, so the sampler never outlives its reference.
static CONTEXT: OnceLock<(JavaVM, Global<JObject<'static>>)> = OnceLock::new();
static LATEST: Mutex<Option<BatteryState>> = Mutex::new(None);
static START: Once = Once::new();

/// Take a global ref to the Application context while `activity` is alive.
/// Sampling starts on the first [`latest`] call.
pub fn init(vm: *mut std::ffi::c_void, activity: *mut std::ffi::c_void) {
    if CONTEXT.get().is_some() {
        return;
    }
    let vm = unsafe { JavaVM::from_raw(vm.cast()) };
    let app = vm.attach_current_thread(|env| -> jni::errors::Result<_> {
        let activity = unsafe { JObject::from_raw(env, activity.cast()) };
        let app = env
            .call_method(
                &activity,
                jni_str!("getApplicationContext"),
                jni_sig!("()Landroid/content/Context;"),
                &[],
            )?
            .l()?;
        env.new_global_ref(app)
    });
    match app {
        Ok(app) => {
            let _ = CONTEXT.set((vm, app));
        }
        Err(e) => log::warn!("battery: no application context: {e}"),
    }
}

/// The cached reading. The first call starts the 1 Hz sampler, so it (and
/// any call before the first sample lands) returns `None`; so does a failed
/// sample, rather than serving a stale one.
pub fn latest() -> Option<BatteryState> {
    START.call_once(|| {
        if let Some((vm, app)) = CONTEXT.get() {
            if let Err(e) = std::thread::Builder::new()
                .name("functor-battery".into())
                .spawn(move || sample_forever(vm, app.as_obj()))
            {
                log::warn!("battery: sampler thread failed to start: {e}");
            }
        }
    });
    LATEST.lock().ok()?.clone()
}

fn sample_forever(vm: &JavaVM, context: &JObject<'static>) {
    let mut warned = false;
    loop {
        // Each attach call runs in its own local frame and catches (and
        // clears) any Java exception as an error.
        let reading = match vm.attach_current_thread(|env| read(env, context)) {
            Ok(raw) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                Some(BatteryState::from_android(raw, now))
            }
            Err(e) => {
                if !warned {
                    log::warn!("battery: read failed: {e}");
                    warned = true;
                }
                None
            }
        };
        if let Ok(mut latest) = LATEST.lock() {
            *latest = reading;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn read(env: &mut Env, context: &JObject) -> jni::errors::Result<AndroidBatteryRaw> {
    let service = env.new_string("batterymanager")?;
    let manager = env
        .call_method(
            context,
            jni_str!("getSystemService"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::Object(&service)],
        )?
        .l()?;
    // BatteryManager.BATTERY_PROPERTY_*; unsupported reads Integer.MIN_VALUE.
    let mut property = |id: i32| -> jni::errors::Result<i32> {
        env.call_method(
            &manager,
            jni_str!("getIntProperty"),
            jni_sig!("(I)I"),
            &[JValue::Int(id)],
        )?
        .i()
    };
    let charge_counter_uah = property(1)?;
    let current_now_ua = property(2)?;
    let current_avg_ua = property(3)?;
    let capacity = property(4)?;

    // The sticky ACTION_BATTERY_CHANGED intent; a null receiver only reads it.
    let action = env.new_string("android.intent.action.BATTERY_CHANGED")?;
    let filter = env.new_object(
        jni_str!("android/content/IntentFilter"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&action)],
    )?;
    let intent = env
        .call_method(
            context,
            jni_str!("registerReceiver"),
            jni_sig!("(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;"),
            &[JValue::Object(&JObject::null()), JValue::Object(&filter)],
        )?
        .l()?;
    let mut extra = |key: &str| -> jni::errors::Result<i32> {
        if intent.is_null() {
            return Ok(i32::MIN);
        }
        let key = env.new_string(key)?;
        env.call_method(
            &intent,
            jni_str!("getIntExtra"),
            jni_sig!("(Ljava/lang/String;I)I"),
            &[JValue::Object(&key), JValue::Int(i32::MIN)],
        )?
        .i()
    };
    Ok(AndroidBatteryRaw {
        capacity,
        current_now_ua,
        current_avg_ua,
        charge_counter_uah,
        level: extra("level")?,
        scale: extra("scale")?,
        status: extra("status")?,
        plugged: extra("plugged")?,
        voltage_mv: extra("voltage")?,
        temperature_dc: extra("temperature")?,
    })
}
