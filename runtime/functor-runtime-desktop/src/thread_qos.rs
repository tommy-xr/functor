//! The sim thread's scheduling class. macOS gives a spawned thread the
//! default QoS, which may schedule it on efficiency cores; the sim is on the
//! frame's critical path, so it asks for the class the main (render) thread
//! already has. Other platforms keep their default for now.

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
    // SAFETY: out-pointers to locals, valid for the call.
    unsafe {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut priority = 0;
        if libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority) == 0 {
            use libc::qos_class_t::*;
            return match class {
                QOS_CLASS_USER_INTERACTIVE => "user-interactive",
                QOS_CLASS_USER_INITIATED => "user-initiated",
                QOS_CLASS_DEFAULT => "default",
                QOS_CLASS_UTILITY => "utility",
                QOS_CLASS_BACKGROUND => "background",
                QOS_CLASS_UNSPECIFIED => "unspecified",
            }
            .to_string();
        }
    }
    "default".to_string()
}
