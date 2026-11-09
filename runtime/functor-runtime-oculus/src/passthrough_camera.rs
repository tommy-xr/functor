//! `Xr.streamCamera`'s source on Quest: the left passthrough RGB camera
//! through the NDK Camera2 API (no Java), read as `YUV_420_888` and
//! repacked as I420.
//!
//! Horizon selects the passthrough cameras by Meta vendor tags in each
//! camera's characteristics. Opening one needs the
//! `horizonos.permission.HEADSET_CAMERA` runtime permission, which this shell
//! does not request yet: grant it once with `adb shell pm grant
//! dev.functor.runner horizonos.permission.HEADSET_CAMERA`.

use std::ffi::{c_void, CStr};
use std::ptr;

use cgmath::Matrix4;
use functor_runtime_common::camera_frame::{
    lens_to_head, rescale_intrinsics, yuv420_888_to_i420, Intrinsics, YuvPlane,
};
use ndk_sys as ffi;

// ndk-sys binds Camera2 without linking it.
#[link(name = "camera2ndk")]
extern "C" {}

/// `com.meta.extra_metadata.camera_source`: 0 = passthrough RGB.
const META_CAMERA_SOURCE: u32 = 0x8000_4d00;
/// `com.meta.extra_metadata.position`: 0 = left, 1 = right (as worn).
const META_CAMERA_POSITION: u32 = 0x8000_4d01;
/// The size to stream, when the camera offers it.
const PREFERRED_SIZE: (i32, i32) = (1280, 960);

/// What the camera's static characteristics say about its images.
#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    pub width: u32,
    pub height: u32,
    pub intrinsics: Intrinsics,
    pub distortion: Option<[f32; 5]>,
    /// OpenCV camera → head (OpenXR VIEW).
    pub camera_to_head: Matrix4<f32>,
}

struct Open {
    manager: *mut ffi::ACameraManager,
    device: *mut ffi::ACameraDevice,
    reader: *mut ffi::AImageReader,
    container: *mut ffi::ACaptureSessionOutputContainer,
    output: *mut ffi::ACaptureSessionOutput,
    target: *mut ffi::ACameraOutputTarget,
    request: *mut ffi::ACaptureRequest,
    session: *mut ffi::ACameraCaptureSession,
    calibration: Calibration,
}

/// The left passthrough camera, open while a stream wants it.
#[derive(Default)]
pub struct PassthroughCamera {
    open: Option<Open>,
    /// Why the camera is not open, while wanted.
    error: Option<String>,
    wanted: bool,
}

/// One image held until dropped.
pub struct CameraImage<'a> {
    image: *mut ffi::AImage,
    pub calibration: &'a Calibration,
}

impl PassthroughCamera {
    /// Open the camera while `wanted`, close it otherwise. A failed open is
    /// retried only after `wanted` goes false and back.
    pub fn update(&mut self, wanted: bool) {
        if wanted && !self.wanted {
            match unsafe { open() } {
                Ok(open) => {
                    log::info!("passthrough camera open: {:?}", open.calibration);
                    self.open = Some(open);
                    self.error = None;
                }
                Err(e) => {
                    log::warn!("Xr.streamCamera: {e}");
                    self.error = Some(e);
                }
            }
        }
        if !wanted {
            if let Some(open) = self.open.take() {
                unsafe { close(open) };
            }
            self.error = None;
        }
        self.wanted = wanted;
    }

    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// The newest image since the last call, if any; older ones are dropped.
    pub fn acquire(&mut self) -> Option<CameraImage<'_>> {
        let open = self.open.as_ref()?;
        let mut image = ptr::null_mut();
        let status = unsafe { ffi::AImageReader_acquireLatestImage(open.reader, &mut image) };
        (status == ffi::media_status_t::AMEDIA_OK && !image.is_null()).then_some(CameraImage {
            image,
            calibration: &open.calibration,
        })
    }
}

impl Drop for PassthroughCamera {
    fn drop(&mut self) {
        self.update(false);
    }
}

impl CameraImage<'_> {
    /// The start of exposure, `CLOCK_MONOTONIC` nanoseconds.
    pub fn sensor_ns(&self) -> Option<i64> {
        let mut ns = 0;
        let status = unsafe { ffi::AImage_getTimestamp(self.image, &mut ns) };
        (status == ffi::media_status_t::AMEDIA_OK).then_some(ns)
    }

    /// The image as I420; `None` if a plane cannot be read.
    pub fn to_i420(&self) -> Option<Vec<u8>> {
        let plane = |index| unsafe {
            let (mut data, mut len, mut row, mut pixel) = (ptr::null_mut(), 0, 0, 0);
            let ok = ffi::AImage_getPlaneData(self.image, index, &mut data, &mut len)
                == ffi::media_status_t::AMEDIA_OK
                && ffi::AImage_getPlaneRowStride(self.image, index, &mut row)
                    == ffi::media_status_t::AMEDIA_OK
                && ffi::AImage_getPlanePixelStride(self.image, index, &mut pixel)
                    == ffi::media_status_t::AMEDIA_OK
                && !data.is_null();
            ok.then(|| YuvPlane {
                data: std::slice::from_raw_parts(data, len as usize),
                row_stride: row as usize,
                pixel_stride: pixel as usize,
            })
        };
        let c = self.calibration;
        Some(yuv420_888_to_i420(c.width, c.height, [plane(0)?, plane(1)?, plane(2)?]))
    }
}

impl Drop for CameraImage<'_> {
    fn drop(&mut self) {
        unsafe { ffi::AImage_delete(self.image) };
    }
}

fn check(what: &str, status: ffi::camera_status_t) -> Result<(), String> {
    match status {
        ffi::camera_status_t::ACAMERA_OK => Ok(()),
        ffi::camera_status_t::ACAMERA_ERROR_PERMISSION_DENIED => Err(format!(
            "{what}: permission denied — grant it with `adb shell pm grant dev.functor.runner \
horizonos.permission.HEADSET_CAMERA`, then restart"
        )),
        status => Err(format!("{what}: camera status {}", status.0)),
    }
}

/// A characteristics entry, if present with `count` at least `min`.
unsafe fn entry(
    metadata: *const ffi::ACameraMetadata,
    tag: u32,
    min: u32,
) -> Option<ffi::ACameraMetadata_const_entry> {
    let mut entry = std::mem::zeroed::<ffi::ACameraMetadata_const_entry>();
    (ffi::ACameraMetadata_getConstEntry(metadata, tag, &mut entry) == ffi::camera_status_t::ACAMERA_OK
        && entry.count >= min)
        .then_some(entry)
}

unsafe fn floats<const N: usize>(metadata: *const ffi::ACameraMetadata, tag: u32) -> Option<[f32; N]> {
    let e = entry(metadata, tag, N as u32)?;
    Some(std::array::from_fn(|i| *e.data.f.add(i)))
}

/// The id of the left passthrough camera and its characteristics (the
/// caller frees them).
unsafe fn find_left(
    manager: *mut ffi::ACameraManager,
) -> Result<(std::ffi::CString, *mut ffi::ACameraMetadata), String> {
    let mut list = ptr::null_mut();
    check("list cameras", ffi::ACameraManager_getCameraIdList(manager, &mut list))?;
    let ids: Vec<std::ffi::CString> = (0..(*list).numCameras as usize)
        .map(|i| CStr::from_ptr(*(*list).cameraIds.add(i)).to_owned())
        .collect();
    ffi::ACameraManager_deleteCameraIdList(list);
    for id in &ids {
        let mut metadata = ptr::null_mut();
        if ffi::ACameraManager_getCameraCharacteristics(manager, id.as_ptr(), &mut metadata)
            != ffi::camera_status_t::ACAMERA_OK
        {
            continue;
        }
        let byte = |tag| entry(metadata, tag, 1).map(|e| *e.data.u8_);
        if byte(META_CAMERA_SOURCE) == Some(0) && byte(META_CAMERA_POSITION) == Some(0) {
            return Ok((id.clone(), metadata));
        }
        ffi::ACameraMetadata_free(metadata);
    }
    Err(format!("no left passthrough camera among {ids:?}"))
}

/// The YUV output size: [`PREFERRED_SIZE`] if offered, else the largest.
unsafe fn output_size(metadata: *const ffi::ACameraMetadata) -> Option<(i32, i32)> {
    let e = entry(
        metadata,
        ffi::acamera_metadata_tag::ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS.0,
        4,
    )?;
    // [format, width, height, is_input] quadruples.
    let configs = std::slice::from_raw_parts(e.data.i32_, e.count as usize);
    let sizes: Vec<(i32, i32)> = configs
        .chunks_exact(4)
        .filter(|c| c[0] == ffi::AIMAGE_FORMATS::AIMAGE_FORMAT_YUV_420_888.0 as i32 && c[3] == 0)
        .map(|c| (c[1], c[2]))
        .collect();
    sizes
        .iter()
        .copied()
        .find(|s| *s == PREFERRED_SIZE)
        .or_else(|| sizes.iter().copied().max_by_key(|(w, h)| w * h))
}

unsafe fn calibration(metadata: *const ffi::ACameraMetadata, size: (i32, i32)) -> Result<Calibration, String> {
    use ffi::acamera_metadata_tag as tag;
    let missing = |what| format!("the left camera reports no {what}");
    let k: [f32; 5] = floats(metadata, tag::ACAMERA_LENS_INTRINSIC_CALIBRATION.0)
        .ok_or_else(|| missing("LENS_INTRINSIC_CALIBRATION"))?;
    // [x, y, z, w]: sensor (head) → camera.
    let q: [f32; 4] =
        floats(metadata, tag::ACAMERA_LENS_POSE_ROTATION.0).ok_or_else(|| missing("LENS_POSE_ROTATION"))?;
    let t: [f32; 3] = floats(metadata, tag::ACAMERA_LENS_POSE_TRANSLATION.0)
        .ok_or_else(|| missing("LENS_POSE_TRANSLATION"))?;
    let array = entry(metadata, tag::ACAMERA_SENSOR_INFO_PRE_CORRECTION_ACTIVE_ARRAY_SIZE.0, 4)
        .ok_or_else(|| missing("SENSOR_INFO_PRE_CORRECTION_ACTIVE_ARRAY_SIZE"))?;
    // [left, top, width, height].
    let array = std::slice::from_raw_parts(array.data.i32_, 4);
    let distortion = floats::<5>(metadata, tag::ACAMERA_LENS_DISTORTION.0)
        .filter(|d| d.iter().any(|c| *c != 0.0));
    let (width, height) = (size.0 as u32, size.1 as u32);
    Ok(Calibration {
        width,
        height,
        intrinsics: rescale_intrinsics(
            [k[0], k[1], k[2], k[3]],
            (array[2] as u32, array[3] as u32),
            (width, height),
        ),
        distortion,
        camera_to_head: lens_to_head(q, t),
    })
}

unsafe extern "C" fn on_disconnected(_: *mut c_void, _: *mut ffi::ACameraDevice) {
    log::warn!("passthrough camera disconnected");
}

unsafe extern "C" fn on_error(_: *mut c_void, _: *mut ffi::ACameraDevice, error: i32) {
    log::warn!("passthrough camera error {error}");
}

unsafe extern "C" fn on_session_state(_: *mut c_void, _: *mut ffi::ACameraCaptureSession) {}

/// Open the left camera into an image reader, repeating a preview request.
unsafe fn open() -> Result<Open, String> {
    let mut open = Open {
        manager: ffi::ACameraManager_create(),
        device: ptr::null_mut(),
        reader: ptr::null_mut(),
        container: ptr::null_mut(),
        output: ptr::null_mut(),
        target: ptr::null_mut(),
        request: ptr::null_mut(),
        session: ptr::null_mut(),
        calibration: Calibration {
            width: 0,
            height: 0,
            intrinsics: Intrinsics {
                fx: 0.0,
                fy: 0.0,
                cx: 0.0,
                cy: 0.0,
            },
            distortion: None,
            camera_to_head: Matrix4::from_scale(1.0),
        },
    };
    // On any failure, `close` frees what was made.
    let result = (|| {
        let (id, metadata) = find_left(open.manager)?;
        let size = output_size(metadata);
        let calibrated = size
            .ok_or_else(|| "the left camera offers no YUV_420_888 output".to_string())
            .and_then(|size| calibration(metadata, size));
        ffi::ACameraMetadata_free(metadata);
        open.calibration = calibrated?;
        let (w, h) = (open.calibration.width as i32, open.calibration.height as i32);
        if ffi::AImageReader_new(w, h, ffi::AIMAGE_FORMATS::AIMAGE_FORMAT_YUV_420_888.0 as i32, 3, &mut open.reader)
            != ffi::media_status_t::AMEDIA_OK
        {
            return Err(format!("AImageReader_new {w}x{h} failed"));
        }
        let mut window = ptr::null_mut();
        if ffi::AImageReader_getWindow(open.reader, &mut window) != ffi::media_status_t::AMEDIA_OK {
            return Err("AImageReader_getWindow failed".to_string());
        }
        let mut device_callbacks = ffi::ACameraDevice_StateCallbacks {
            context: ptr::null_mut(),
            onDisconnected: Some(on_disconnected),
            onError: Some(on_error),
        };
        check(
            "open the passthrough camera",
            ffi::ACameraManager_openCamera(open.manager, id.as_ptr(), &mut device_callbacks, &mut open.device),
        )?;
        check("output container", ffi::ACaptureSessionOutputContainer_create(&mut open.container))?;
        check("session output", ffi::ACaptureSessionOutput_create(window, &mut open.output))?;
        check("add output", ffi::ACaptureSessionOutputContainer_add(open.container, open.output))?;
        let session_callbacks = ffi::ACameraCaptureSession_stateCallbacks {
            context: ptr::null_mut(),
            onClosed: Some(on_session_state),
            onReady: Some(on_session_state),
            onActive: Some(on_session_state),
        };
        check(
            "capture session",
            ffi::ACameraDevice_createCaptureSession(open.device, open.container, &session_callbacks, &mut open.session),
        )?;
        check(
            "capture request",
            ffi::ACameraDevice_createCaptureRequest(
                open.device,
                ffi::ACameraDevice_request_template::TEMPLATE_PREVIEW,
                &mut open.request,
            ),
        )?;
        check("output target", ffi::ACameraOutputTarget_create(window, &mut open.target))?;
        check("add target", ffi::ACaptureRequest_addTarget(open.request, open.target))?;
        check(
            "repeating request",
            ffi::ACameraCaptureSession_setRepeatingRequest(
                open.session,
                ptr::null_mut(),
                1,
                &mut open.request,
                ptr::null_mut(),
            ),
        )
    })();
    match result {
        Ok(()) => Ok(open),
        Err(e) => {
            close(open);
            Err(e)
        }
    }
}

/// Stop and free everything `open` made, in reverse.
unsafe fn close(open: Open) {
    if !open.session.is_null() {
        ffi::ACameraCaptureSession_stopRepeating(open.session);
        ffi::ACameraCaptureSession_close(open.session);
    }
    if !open.request.is_null() {
        ffi::ACaptureRequest_free(open.request);
    }
    if !open.target.is_null() {
        ffi::ACameraOutputTarget_free(open.target);
    }
    if !open.device.is_null() {
        ffi::ACameraDevice_close(open.device);
    }
    if !open.output.is_null() {
        ffi::ACaptureSessionOutput_free(open.output);
    }
    if !open.container.is_null() {
        ffi::ACaptureSessionOutputContainer_free(open.container);
    }
    if !open.reader.is_null() {
        ffi::AImageReader_delete(open.reader);
    }
    ffi::ACameraManager_delete(open.manager);
}
