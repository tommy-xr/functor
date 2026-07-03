//! Xreal One 3DoF head tracking for `--xreal-tracking`.
//!
//! The Xreal One (base and Pro) exposes a USB CDC-NCM network interface and
//! streams IMU packets continuously (~1kHz) from `169.254.2.1:52998` — plain
//! TCP, no handshake. Protocol shape verified against the glasses and matches
//! rohitsangwan01/xreal_one_driver and wheaney's XRLinuxDriver
//! (`imu_protocol_xo`). Little-endian, offsets from the start of a message
//! (a message starts at the 6-byte magic):
//!
//!   magic  `28 36 00 00 00 80`      @ 0
//!   timestamp u64, nanoseconds      @ 14
//!   gyro   f32 x,y,z  rad/s         @ 34
//!   accel  f32 x,y,z  m/s²          @ 46
//!
//! Pipeline: [`PacketScanner`] (bytes → samples) → gyro-bias calibration
//! (first `CALIBRATION_SAMPLES`, glasses assumed still) → [`Fusion`]
//! (Mahony-style complementary filter, gravity-corrected pitch/roll; yaw is
//! gyro-only and drifts slowly — F1 recenters) → a shared quaternion the
//! render loop reads once per frame and applies with
//! [`apply_head_rotation`].
//!
//! The glasses' own stabilizer/anchor must be OFF (on-glasses OSD) — it
//! consumes the IMU internally and its display-space warp fights ours.

use std::io::Read;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cgmath::{InnerSpace, Quaternion, Rotation, Vector3, Zero};
use functor_runtime_common::Camera;

pub const DEFAULT_ADDR: &str = "169.254.2.1:52998";

const MAGIC: [u8; 6] = [0x28, 0x36, 0x00, 0x00, 0x00, 0x80];
/// A full message spans at least this many bytes from the magic; the fields
/// we read all sit inside it.
const MESSAGE_LEN: usize = 84;
const TIMESTAMP_OFFSET: usize = 14;
const GYRO_OFFSET: usize = 34;
const ACCEL_OFFSET: usize = 46;

/// One IMU reading remapped into the *head frame*: x = right, y = up,
/// z = backward — the right-handed OpenGL camera convention, so the wearer's
/// gaze is −z. (x-right/y-up/z-*forward* would be left-handed and silently
/// flip every rotation under RH quaternion math.) Angular velocity in rad/s,
/// specific force in m/s² (reads +1g on +y when worn level at rest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImuSample {
    pub timestamp_ns: u64,
    pub gyro: Vector3<f32>,
    pub accel: Vector3<f32>,
}

/// Map the wire-order sensor axes into the head frame (see [`ImuSample`]).
/// This is the ONE place axis conventions live — a best guess derived from
/// the community drivers' remap (`[-x, -z, -y]` into their z-forward frame)
/// with z negated for our z-backward frame; tune signs here if a live
/// rotation reads backwards.
fn remap(v: [f32; 3]) -> Vector3<f32> {
    Vector3::new(-v[0], -v[2], v[1])
}

/// Incremental scanner: feed raw TCP bytes, get parsed samples. Handles
/// messages split across reads and garbage between messages (it hunts for the
/// magic). Non-finite samples are dropped.
pub struct PacketScanner {
    buf: Vec<u8>,
}

impl PacketScanner {
    pub fn new() -> PacketScanner {
        PacketScanner { buf: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<ImuSample> {
        self.buf.extend_from_slice(bytes);
        let mut samples = Vec::new();
        let mut pos = 0;
        while let Some(start) = find_magic(&self.buf[pos..]).map(|i| pos + i) {
            if self.buf.len() - start < MESSAGE_LEN {
                // Incomplete message: keep from the magic onward for the next read.
                pos = start;
                break;
            }
            if let Some(sample) = parse_at(&self.buf, start) {
                samples.push(sample);
            }
            // Fields end well before MESSAGE_LEN; resume the magic hunt right
            // after the parsed fields so a shorter-than-expected next message
            // can't be skipped.
            pos = start + ACCEL_OFFSET + 12;
        }
        // Nothing before `pos` can start a complete message anymore, except a
        // trailing partial magic — keep a small tail so a magic split across
        // reads still matches.
        let keep_from = pos.min(self.buf.len().saturating_sub(MESSAGE_LEN));
        self.buf.drain(..keep_from);
        samples
    }
}

fn find_magic(buf: &[u8]) -> Option<usize> {
    buf.windows(MAGIC.len()).position(|w| w == MAGIC)
}

fn parse_at(buf: &[u8], start: usize) -> Option<ImuSample> {
    let f32_at = |off: usize| {
        let b = &buf[start + off..start + off + 4];
        f32::from_le_bytes([b[0], b[1], b[2], b[3]])
    };
    let ts = {
        let b = &buf[start + TIMESTAMP_OFFSET..start + TIMESTAMP_OFFSET + 8];
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    };
    let gyro = remap([f32_at(GYRO_OFFSET), f32_at(GYRO_OFFSET + 4), f32_at(GYRO_OFFSET + 8)]);
    let accel = remap([
        f32_at(ACCEL_OFFSET),
        f32_at(ACCEL_OFFSET + 4),
        f32_at(ACCEL_OFFSET + 8),
    ]);
    let finite =
        |v: Vector3<f32>| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if !finite(gyro) || !finite(accel) {
        return None;
    }
    Some(ImuSample {
        timestamp_ns: ts,
        gyro,
        accel,
    })
}

/// Mahony-style complementary filter. Gyro integration gives responsiveness;
/// a small proportional pull toward the accelerometer's gravity direction
/// keeps pitch/roll from drifting. Yaw has no absolute reference (no
/// magnetometer) and drifts slowly — recentering handles it.
pub struct Fusion {
    /// Head orientation: rotates head-frame vectors into the world frame.
    q: Quaternion<f32>,
    gyro_bias: Vector3<f32>,
    last_timestamp_ns: Option<u64>,
}

/// Proportional gain of the gravity correction (1/s). Higher = faster
/// leveling but more accel noise/motion leaking into orientation.
const ACCEL_GAIN: f32 = 1.5;
/// Only trust the accelerometer as a gravity reference when its magnitude is
/// near 1g — during shakes/turns it measures motion, not gravity.
const GRAVITY_MIN: f32 = 7.8; // m/s²
const GRAVITY_MAX: f32 = 11.8;
/// Ignore nonsense dt from timestamp glitches (stream is ~1ms).
const MAX_DT: f32 = 0.05;

impl Fusion {
    pub fn new(gyro_bias: Vector3<f32>) -> Fusion {
        Fusion {
            q: Quaternion::new(1.0, 0.0, 0.0, 0.0),
            gyro_bias,
            last_timestamp_ns: None,
        }
    }

    pub fn orientation(&self) -> Quaternion<f32> {
        self.q
    }

    pub fn update(&mut self, sample: &ImuSample) {
        let dt = match self.last_timestamp_ns {
            Some(last) if sample.timestamp_ns > last => {
                ((sample.timestamp_ns - last) as f64 / 1e9) as f32
            }
            _ => 0.0,
        };
        self.last_timestamp_ns = Some(sample.timestamp_ns);
        if dt <= 0.0 || dt > MAX_DT {
            return;
        }

        let mut omega = sample.gyro - self.gyro_bias;

        // Gravity correction: the accelerometer at rest reads +1g along
        // world-up. Compare measured up (head frame) with where the current
        // orientation says world-up is (world-up brought into head frame);
        // their cross product is the small rotation, in the head frame, that
        // would align them — feed it in as extra angular velocity.
        let a_mag = sample.accel.magnitude();
        if (GRAVITY_MIN..=GRAVITY_MAX).contains(&a_mag) {
            let measured_up = sample.accel / a_mag;
            let estimated_up = self.q.invert().rotate_vector(Vector3::unit_y());
            omega += measured_up.cross(estimated_up) * ACCEL_GAIN;
        }

        // Integrate: q̇ = ½ · q · ω (ω as a pure quaternion in the head frame).
        let dq = self.q * Quaternion::from_sv(0.0, omega) * 0.5;
        self.q = (self.q + dq * dt).normalize();
    }
}

/// Average the gyro over the calibration window to estimate its at-rest bias
/// (the Xreal One shows ~0.9°/s on one axis). The glasses are assumed still —
/// they're usually on the desk when the runner starts.
pub const CALIBRATION_SAMPLES: usize = 500; // ~0.5s at the stream's ~1kHz

pub fn gyro_bias(samples: &[ImuSample]) -> Vector3<f32> {
    if samples.is_empty() {
        return Vector3::zero();
    }
    samples.iter().map(|s| s.gyro).sum::<Vector3<f32>>() / samples.len() as f32
}

/// Rotate a game camera by the (recentered) head orientation. `q` is in the
/// head frame — x right, y up, z forward — so it's applied in the *camera's
/// local basis*: whatever direction the game camera faces, looking left turns
/// the view left. Falls back to the unrotated camera for a degenerate gaze
/// (same guard as `Camera::stereo_eyes`).
pub fn apply_head_rotation(camera: &Camera, q: Quaternion<f32>) -> Camera {
    let eye = Vector3::new(camera.eye[0], camera.eye[1], camera.eye[2]);
    let target = Vector3::new(camera.target[0], camera.target[1], camera.target[2]);
    let up = Vector3::new(camera.up[0], camera.up[1], camera.up[2]);

    let gaze = target - eye;
    let dist = gaze.magnitude();
    let right = gaze.cross(up);
    if !dist.is_normal() || !right.magnitude().is_normal() {
        return camera.clone();
    }
    let f = gaze / dist;
    let r = right.normalize();
    let u = r.cross(f).normalize();
    // Right-handed camera basis (r, u, b) with b = backward — matching the
    // head frame — so quaternion rotations keep their sign when mapped
    // through it. (Using forward as the third axis would make the basis
    // left-handed and mirror every head turn.)
    let b = -f;

    // Head-local gaze (−z) and up, rotated by the head orientation…
    let f_local = q.rotate_vector(-Vector3::unit_z());
    let u_local = q.rotate_vector(Vector3::unit_y());
    // …mapped back into the world through the camera basis.
    let to_world = |v: Vector3<f32>| r * v.x + u * v.y + b * v.z;
    let new_f = to_world(f_local);
    let new_u = to_world(u_local);

    let new_target = eye + new_f * dist;
    Camera {
        target: [new_target.x, new_target.y, new_target.z],
        up: [new_u.x, new_u.y, new_u.z],
        ..camera.clone()
    }
}

/// Handle to the background reader thread. The thread owns the TCP
/// connection, calibration, and fusion; the render loop only reads the
/// latest orientation (relative to the last recenter) once per frame.
pub struct XrealTracker {
    orientation: Arc<Mutex<Quaternion<f32>>>,
    recenter: Arc<AtomicBool>,
}

impl XrealTracker {
    /// Spawn the reader. Connection failures don't fail the runner — the
    /// thread retries every 2s and logs transitions, so you can plug the
    /// glasses in after launch.
    pub fn spawn(addr: String) -> XrealTracker {
        let orientation = Arc::new(Mutex::new(Quaternion::new(1.0, 0.0, 0.0, 0.0)));
        let recenter = Arc::new(AtomicBool::new(false));
        let shared = orientation.clone();
        let recenter_flag = recenter.clone();
        std::thread::spawn(move || reader_loop(&addr, &shared, &recenter_flag));
        XrealTracker {
            orientation,
            recenter,
        }
    }

    /// Head orientation relative to the last recenter (identity until the
    /// stream is up and calibrated).
    pub fn orientation(&self) -> Quaternion<f32> {
        *self.orientation.lock().unwrap()
    }

    /// Make the current head pose the new "looking straight ahead".
    pub fn request_recenter(&self) {
        self.recenter.store(true, Ordering::Relaxed);
    }
}

fn reader_loop(
    addr: &str,
    shared: &Mutex<Quaternion<f32>>,
    recenter_flag: &AtomicBool,
) {
    loop {
        match TcpStream::connect_timeout(
            &addr.parse().expect("invalid --xreal-addr"),
            Duration::from_secs(2),
        ) {
            Ok(mut stream) => {
                println!("[xreal] connected to {addr}; calibrating (keep the glasses still)…");
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                read_stream(&mut stream, shared, recenter_flag);
                println!("[xreal] stream ended; reconnecting…");
                // The old orientation would be stale after a gap; hold the
                // last pose (identity jump on reconnect is worse).
            }
            Err(e) => {
                println!("[xreal] connect to {addr} failed ({e}); retrying in 2s (are the glasses plugged in?)");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Pump one connection: calibrate, then fuse and publish until it drops.
fn read_stream(
    stream: &mut TcpStream,
    shared: &Mutex<Quaternion<f32>>,
    recenter_flag: &AtomicBool,
) {
    let mut scanner = PacketScanner::new();
    let mut calibration: Vec<ImuSample> = Vec::with_capacity(CALIBRATION_SAMPLES);
    let mut fusion: Option<Fusion> = None;
    // Reference pose: orientation published to the game is q_ref⁻¹ · q, so
    // recentering just captures the current absolute orientation.
    let mut q_ref = Quaternion::new(1.0, 0.0, 0.0, 0.0);
    let mut buf = [0u8; 4096];

    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        for sample in scanner.push(&buf[..n]) {
            match &mut fusion {
                None => {
                    calibration.push(sample);
                    if calibration.len() >= CALIBRATION_SAMPLES {
                        let bias = gyro_bias(&calibration);
                        println!(
                            "[xreal] calibrated: gyro bias [{:+.4} {:+.4} {:+.4}] rad/s — tracking live (F1 recenters)",
                            bias.x, bias.y, bias.z
                        );
                        fusion = Some(Fusion::new(bias));
                    }
                }
                Some(fusion) => {
                    fusion.update(&sample);
                    if recenter_flag.swap(false, Ordering::Relaxed) {
                        q_ref = fusion.orientation();
                    }
                    *shared.lock().unwrap() = q_ref.invert() * fusion.orientation();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, Rotation3};

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// Build one wire message: magic + fields at the documented offsets,
    /// padded to MESSAGE_LEN. Values are in *wire* (pre-remap) axis order.
    fn message(ts: u64, gyro: [f32; 3], accel: [f32; 3]) -> Vec<u8> {
        let mut m = vec![0u8; MESSAGE_LEN];
        m[..6].copy_from_slice(&MAGIC);
        m[TIMESTAMP_OFFSET..TIMESTAMP_OFFSET + 8].copy_from_slice(&ts.to_le_bytes());
        for i in 0..3 {
            m[GYRO_OFFSET + i * 4..GYRO_OFFSET + i * 4 + 4]
                .copy_from_slice(&gyro[i].to_le_bytes());
            m[ACCEL_OFFSET + i * 4..ACCEL_OFFSET + i * 4 + 4]
                .copy_from_slice(&accel[i].to_le_bytes());
        }
        m
    }

    #[test]
    fn scanner_parses_message_with_leading_garbage_and_remaps_axes() {
        let mut scanner = PacketScanner::new();
        let mut bytes = vec![0xAB, 0xCD, 0x28, 0x36]; // garbage incl. a fake magic prefix
        bytes.extend(message(1_000_000, [1.0, 2.0, 3.0], [0.0, 0.0, -9.8]));
        let samples = scanner.push(&bytes);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].timestamp_ns, 1_000_000);
        // remap [x,y,z] -> [-x,-z,+y]
        assert_eq!(samples[0].gyro, Vector3::new(-1.0, -3.0, 2.0));
        assert_eq!(samples[0].accel, Vector3::new(-0.0, 9.8, 0.0));
    }

    #[test]
    fn scanner_reassembles_message_split_across_reads() {
        let mut scanner = PacketScanner::new();
        let msg = message(42, [0.1, 0.2, 0.3], [0.0, 0.0, -9.8]);
        let (a, b) = msg.split_at(20); // split inside the header
        assert!(scanner.push(a).is_empty());
        let samples = scanner.push(b);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].timestamp_ns, 42);
    }

    #[test]
    fn scanner_parses_back_to_back_messages_and_drops_non_finite() {
        let mut scanner = PacketScanner::new();
        let mut bytes = message(1, [0.1, 0.0, 0.0], [0.0, 0.0, -9.8]);
        bytes.extend(message(2, [f32::NAN, 0.0, 0.0], [0.0, 0.0, -9.8])); // dropped
        bytes.extend(message(3, [0.2, 0.0, 0.0], [0.0, 0.0, -9.8]));
        let samples = scanner.push(&bytes);
        assert_eq!(
            samples.iter().map(|s| s.timestamp_ns).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    /// At rest (accel = wire -z, i.e. head-frame +y = 1g up; zero gyro) the
    /// orientation must stay identity — no drift from the correction term.
    #[test]
    fn fusion_stationary_stays_identity() {
        let mut fusion = Fusion::new(Vector3::zero());
        for i in 0..2000u64 {
            fusion.update(&ImuSample {
                timestamp_ns: i * 1_000_000,
                gyro: Vector3::zero(),
                accel: Vector3::new(0.0, 9.81, 0.0),
            });
        }
        let q = fusion.orientation();
        assert!(approx(q.s, 1.0, 1e-3), "q = {q:?}");
    }

    /// Integrating a constant yaw rate must yield that yaw angle: 90°/s about
    /// head-frame +y for 1s at 1kHz ≈ 90° of yaw.
    #[test]
    fn fusion_integrates_yaw_rate() {
        let mut fusion = Fusion::new(Vector3::zero());
        let rate = std::f32::consts::FRAC_PI_2; // rad/s about +y
        for i in 0..=1000u64 {
            fusion.update(&ImuSample {
                timestamp_ns: i * 1_000_000,
                // Yaw doesn't tilt the gravity vector.
                gyro: Vector3::new(0.0, rate, 0.0),
                accel: Vector3::new(0.0, 9.81, 0.0),
            });
        }
        let expected = Quaternion::from_angle_y(Deg(90.0));
        let q = fusion.orientation();
        assert!(q.dot(expected).abs() > 0.999, "q = {q:?}");
    }

    /// With a tilted gravity vector and silent gyro, the correction must pull
    /// pitch toward the accelerometer's story (leveling after gyro drift).
    #[test]
    fn fusion_accel_corrects_toward_gravity() {
        let mut fusion = Fusion::new(Vector3::zero());
        // Accel says "world up is along head +z" — head pitched down 90°.
        let target_up = Vector3::new(0.0, 0.0, 9.81);
        for i in 0..20000u64 {
            fusion.update(&ImuSample {
                timestamp_ns: i * 1_000_000,
                gyro: Vector3::zero(),
                accel: target_up,
            });
        }
        let estimated_up = fusion
            .orientation()
            .invert()
            .rotate_vector(Vector3::unit_y());
        assert!(
            estimated_up.dot(target_up.normalize()) > 0.99,
            "estimated_up = {estimated_up:?}"
        );
    }

    #[test]
    fn gyro_bias_is_mean_and_empty_is_zero() {
        assert_eq!(gyro_bias(&[]), Vector3::zero());
        let samples: Vec<ImuSample> = [[0.1, 0.0, 0.0], [0.3, 0.0, 0.0]]
            .iter()
            .map(|g| ImuSample {
                timestamp_ns: 0,
                gyro: Vector3::new(g[0], g[1], g[2]),
                accel: Vector3::zero(),
            })
            .collect();
        assert_eq!(gyro_bias(&samples), Vector3::new(0.2, 0.0, 0.0));
    }

    /// Head yaw applied to a first-person camera must equal the camera the
    /// game would build with that yaw — the property that makes head-look
    /// consistent with Functor's yaw/pitch conventions (yaw about +Y; yaw 0
    /// looks down +Z).
    #[test]
    fn head_yaw_matches_first_person_yaw() {
        let base = Camera::look_at(
            [1.0, 2.0, 3.0],
            [1.0, 2.0, 8.0], // looking down +Z, 5 units
            [0.0, 1.0, 0.0],
            functor_runtime_common::math::Angle::from_degrees(45.0),
        );
        let q = Quaternion::from_angle_y(Deg(30.0));
        let rotated = apply_head_rotation(&base, q);

        // Camera::first_person: forward = [cos p · sin yaw, sin p, cos p · cos yaw]
        let yaw = Deg(30.0);
        let expected_f = Vector3::new(
            cgmath::Angle::sin(yaw),
            0.0,
            cgmath::Angle::cos(yaw),
        );
        let got_f = (Vector3::from(rotated.target) - Vector3::from(rotated.eye)).normalize();
        assert!(got_f.dot(expected_f) > 0.9999, "forward = {got_f:?}");
        // Distance to target and eye position preserved.
        assert_eq!(rotated.eye, base.eye);
        assert!(approx(
            (Vector3::from(rotated.target) - Vector3::from(rotated.eye)).magnitude(),
            5.0,
            1e-4
        ));
    }

    #[test]
    fn identity_head_rotation_is_a_noop_and_degenerate_camera_survives() {
        let base = Camera::default();
        let same = apply_head_rotation(&base, Quaternion::new(1.0, 0.0, 0.0, 0.0));
        assert_eq!(same.target, base.target);
        assert_eq!(same.up, base.up);

        // eye == target must not NaN.
        let degenerate = Camera::look_at(
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            functor_runtime_common::math::Angle::from_degrees(45.0),
        );
        let out = apply_head_rotation(&degenerate, Quaternion::from_angle_y(Deg(45.0)));
        assert!(out.target.iter().all(|c| c.is_finite()));
    }
}
