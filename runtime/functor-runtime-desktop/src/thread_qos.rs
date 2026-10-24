//! The sim thread's scheduling class. macOS gives a spawned thread the
//! default QoS, which may schedule it on efficiency cores; the sim is on the
//! frame's critical path, so it asks for the class the main (render) thread
//! has (user-interactive, as `/state.sim_thread` reports inline). Other
//! platforms keep their default for now.

/// Set by `FUNCTOR_THREAD_QOS=off` to leave the sim thread at the platform
/// default (A/B measurement).
fn disabled() -> bool {
    std::env::var("FUNCTOR_THREAD_QOS").is_ok_and(|v| v == "off")
}

/// Ask for user-interactive QoS on the calling thread (macOS).
pub fn raise_current_thread() {
    if disabled() {
        return;
    }
    #[cfg(target_os = "macos")]
    // SAFETY: plain libc call on the calling thread; no pointers involved.
    unsafe {
        let status =
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
        if status != 0 {
            eprintln!("[runner] could not raise the sim thread's QoS (error {status})");
        }
    }
}

/// The calling thread's scheduling class, as `/state.sim_thread` reports it.
pub fn current_label() -> String {
    #[cfg(target_os = "macos")]
    // SAFETY: out-pointers to locals, valid for the call. The class is read
    // as a raw u32: the OS may return values the Rust enum doesn't name.
    unsafe {
        let mut class: u32 = 0;
        let mut priority = 0;
        let out = &mut class as *mut u32 as *mut libc::qos_class_t;
        if libc::pthread_get_qos_class_np(libc::pthread_self(), out, &mut priority) == 0 {
            use libc::qos_class_t::*;
            let name = |c: libc::qos_class_t| c as u32 == class;
            return if name(QOS_CLASS_USER_INTERACTIVE) {
                "user-interactive".to_string()
            } else if name(QOS_CLASS_USER_INITIATED) {
                "user-initiated".to_string()
            } else if name(QOS_CLASS_DEFAULT) {
                "default".to_string()
            } else if name(QOS_CLASS_UTILITY) {
                "utility".to_string()
            } else if name(QOS_CLASS_BACKGROUND) {
                "background".to_string()
            } else {
                format!("qos {class:#x}")
            };
        }
    }
    "platform default".to_string()
}
