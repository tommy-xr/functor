//! `/state.battery`: Android `BatteryManager` read over JNI on a background
//! thread, at most once per second. `current_now` in sysfs is
//! permission-denied over adb on Quest, but the app can read it in-process.

use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use functor_runtime_common::debug_protocol::{AndroidBatteryRaw, BatteryState};
use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};

/// `(JavaVM*, Activity global ref)` as integers so they can cross threads.
static CONTEXT: OnceLock<(usize, usize)> = OnceLock::new();
static LATEST: Mutex<Option<BatteryState>> = Mutex::new(None);
static START: Once = Once::new();

/// Record the JNI handles; sampling starts on the first [`latest`] call.
pub fn init(vm: *mut std::ffi::c_void, activity: *mut std::ffi::c_void) {
    let _ = CONTEXT.set((vm as usize, activity as usize));
}

/// The cached reading. The first call starts the 1 Hz sampler, so it (and
/// any call before the first sample lands) returns `None`.
pub fn latest() -> Option<BatteryState> {
    START.call_once(|| {
        if let Some(&(vm, activity)) = CONTEXT.get() {
            let _ = std::thread::Builder::new()
                .name("functor-battery".into())
                .spawn(move || sample_forever(vm, activity));
        }
    });
    LATEST.lock().ok()?.clone()
}

fn sample_forever(vm: usize, activity: usize) {
    let vm = match unsafe { JavaVM::from_raw(vm as *mut jni::sys::JavaVM) } {
        Ok(vm) => vm,
        Err(e) => return log::warn!("battery: no JavaVM: {e}"),
    };
    let mut env = match vm.attach_current_thread_permanently() {
        Ok(env) => env,
        Err(e) => return log::warn!("battery: attach failed: {e}"),
    };
    // A global ref owned by NativeActivity; never deleted here.
    let context = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
    let mut warned = false;
    loop {
        // A local frame per sample so this never-detaching thread doesn't
        // accumulate local refs.
        match env.with_local_frame(16, |env| read(env, &context)) {
            Ok(raw) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                if let Ok(mut latest) = LATEST.lock() {
                    *latest = Some(BatteryState::from_android(raw, now));
                }
            }
            Err(e) => {
                let _ = env.exception_clear();
                if !warned {
                    log::warn!("battery: read failed: {e}");
                    warned = true;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn read(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<AndroidBatteryRaw> {
    let service = env.new_string("batterymanager")?;
    let manager = env
        .call_method(
            context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )?
        .l()?;
    // BatteryManager.BATTERY_PROPERTY_*; unsupported reads Integer.MIN_VALUE.
    let mut property = |id: i32| -> jni::errors::Result<i32> {
        env.call_method(&manager, "getIntProperty", "(I)I", &[JValue::Int(id)])?
            .i()
    };
    let charge_counter_uah = property(1)?;
    let current_now_ua = property(2)?;
    let current_avg_ua = property(3)?;
    let capacity = property(4)?;

    // The sticky ACTION_BATTERY_CHANGED intent; a null receiver only reads it.
    let action = env.new_string("android.intent.action.BATTERY_CHANGED")?;
    let filter = env.new_object(
        "android/content/IntentFilter",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&action)],
    )?;
    let intent = env
        .call_method(
            context,
            "registerReceiver",
            "(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;",
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
            "getIntExtra",
            "(Ljava/lang/String;I)I",
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
