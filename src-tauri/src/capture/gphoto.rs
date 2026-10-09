//! Cameras on USB through libgphoto2 (LGPL-2.1-or-later).
//!
//! The library is loaded when cameras are first looked for, not linked, so
//! Tonality builds and runs without it; capture then says it is missing.
//! Only the few calls capture needs are declared, from libgphoto2 2.5's
//! headers (`gphoto2-camera.h`, `gphoto2-list.h`, `gphoto2-file.h`,
//! `gphoto2-port-info-list.h`).

use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::{Camera, Found, Shot};

type GpContext = c_void;
type GpCamera = c_void;
type GpList = c_void;
type GpFile = c_void;
type GpPortInfoList = c_void;
/// `GPPortInfo` is a pointer to an opaque struct.
type GpPortInfo = *mut c_void;

/// `CameraFilePath`.
#[repr(C)]
struct FilePath {
    name: [c_char; 128],
    folder: [c_char; 1024],
}

const GP_CAPTURE_IMAGE: c_int = 0;
const GP_FILE_TYPE_NORMAL: c_int = 1;
const GP_EVENT_FILE_ADDED: c_int = 2;

const GP_ERROR: c_int = -1;
const GP_ERROR_IO_USB_FIND: c_int = -52;
const GP_ERROR_IO_USB_CLAIM: c_int = -53;
const GP_ERROR_IO_LOCK: c_int = -60;
const GP_ERROR_MODEL_NOT_FOUND: c_int = -105;
const GP_ERROR_CAMERA_BUSY: c_int = -110;

extern "C" {
    /// Event data is allocated by libgphoto2 with malloc, for the caller to free.
    fn free(pointer: *mut c_void);
}

/// The calls used, looked up once.
struct Gphoto {
    context_new: unsafe extern "C" fn() -> *mut GpContext,
    context_unref: unsafe extern "C" fn(*mut GpContext),
    list_new: unsafe extern "C" fn(*mut *mut GpList) -> c_int,
    list_free: unsafe extern "C" fn(*mut GpList) -> c_int,
    list_count: unsafe extern "C" fn(*mut GpList) -> c_int,
    list_get_name: unsafe extern "C" fn(*mut GpList, c_int, *mut *const c_char) -> c_int,
    list_get_value: unsafe extern "C" fn(*mut GpList, c_int, *mut *const c_char) -> c_int,
    camera_autodetect: unsafe extern "C" fn(*mut GpList, *mut GpContext) -> c_int,
    camera_new: unsafe extern "C" fn(*mut *mut GpCamera) -> c_int,
    camera_set_port_info: unsafe extern "C" fn(*mut GpCamera, GpPortInfo) -> c_int,
    camera_init: unsafe extern "C" fn(*mut GpCamera, *mut GpContext) -> c_int,
    camera_exit: unsafe extern "C" fn(*mut GpCamera, *mut GpContext) -> c_int,
    camera_unref: unsafe extern "C" fn(*mut GpCamera) -> c_int,
    camera_capture: unsafe extern "C" fn(*mut GpCamera, c_int, *mut FilePath, *mut GpContext) -> c_int,
    camera_wait_for_event:
        unsafe extern "C" fn(*mut GpCamera, c_int, *mut c_int, *mut *mut c_void, *mut GpContext) -> c_int,
    camera_file_get:
        unsafe extern "C" fn(*mut GpCamera, *const c_char, *const c_char, c_int, *mut GpFile, *mut GpContext) -> c_int,
    file_new: unsafe extern "C" fn(*mut *mut GpFile) -> c_int,
    file_unref: unsafe extern "C" fn(*mut GpFile) -> c_int,
    file_get_data_and_size: unsafe extern "C" fn(*mut GpFile, *mut *const c_char, *mut c_ulong) -> c_int,
    port_info_list_new: unsafe extern "C" fn(*mut *mut GpPortInfoList) -> c_int,
    port_info_list_load: unsafe extern "C" fn(*mut GpPortInfoList) -> c_int,
    port_info_list_lookup_path: unsafe extern "C" fn(*mut GpPortInfoList, *const c_char) -> c_int,
    port_info_list_get_info: unsafe extern "C" fn(*mut GpPortInfoList, c_int, *mut GpPortInfo) -> c_int,
    port_info_list_free: unsafe extern "C" fn(*mut GpPortInfoList) -> c_int,
    result_as_string: unsafe extern "C" fn(c_int) -> *const c_char,
    /// Kept loaded for as long as the app runs.
    _library: libloading::Library,
}

const MISSING: &str = "Tethered capture needs libgphoto2, which isn’t installed. Install it with your system’s \
                       package manager (the package is usually called libgphoto2), then open Tonality again.";

fn gphoto() -> Result<&'static Gphoto> {
    static LOADED: OnceLock<Option<Gphoto>> = OnceLock::new();
    LOADED.get_or_init(|| load().ok()).as_ref().ok_or_else(|| anyhow!(MISSING))
}

/// Why libgphoto2 can't be used, if it can't.
pub fn unavailable() -> Option<String> {
    gphoto().err().map(|error| error.to_string())
}

fn load() -> Result<Gphoto> {
    // SAFETY: loading libgphoto2 runs its initialisers, which only set up
    // its own state.
    let library = ["libgphoto2.so.6", "libgphoto2.so"]
        .iter()
        .find_map(|name| unsafe { libloading::Library::new(name) }.ok())
        .context("libgphoto2 isn't installed")?;
    macro_rules! get {
        ($name:literal) => {
            // SAFETY: each name is declared with its signature in libgphoto2
            // 2.5's headers; the pointer is used only while `library` is held.
            unsafe { *library.get(concat!($name, "\0").as_bytes())? }
        };
    }
    Ok(Gphoto {
        context_new: get!("gp_context_new"),
        context_unref: get!("gp_context_unref"),
        list_new: get!("gp_list_new"),
        list_free: get!("gp_list_free"),
        list_count: get!("gp_list_count"),
        list_get_name: get!("gp_list_get_name"),
        list_get_value: get!("gp_list_get_value"),
        camera_autodetect: get!("gp_camera_autodetect"),
        camera_new: get!("gp_camera_new"),
        camera_set_port_info: get!("gp_camera_set_port_info"),
        camera_init: get!("gp_camera_init"),
        camera_exit: get!("gp_camera_exit"),
        camera_unref: get!("gp_camera_unref"),
        camera_capture: get!("gp_camera_capture"),
        camera_wait_for_event: get!("gp_camera_wait_for_event"),
        camera_file_get: get!("gp_camera_file_get"),
        file_new: get!("gp_file_new"),
        file_unref: get!("gp_file_unref"),
        file_get_data_and_size: get!("gp_file_get_data_and_size"),
        port_info_list_new: get!("gp_port_info_list_new"),
        port_info_list_load: get!("gp_port_info_list_load"),
        port_info_list_lookup_path: get!("gp_port_info_list_lookup_path"),
        port_info_list_get_info: get!("gp_port_info_list_get_info"),
        port_info_list_free: get!("gp_port_info_list_free"),
        result_as_string: get!("gp_result_as_string"),
        _library: library,
    })
}

impl Gphoto {
    /// A libgphoto2 result as an error in plain words.
    fn check(&self, result: c_int) -> Result<c_int> {
        if result >= 0 {
            return Ok(result);
        }
        let plain = match result {
            GP_ERROR_IO_USB_CLAIM | GP_ERROR_IO_LOCK => {
                "another program is using the camera. Close it (a file manager showing the camera counts) and try again"
            }
            GP_ERROR_MODEL_NOT_FOUND | GP_ERROR_IO_USB_FIND => {
                "no camera found. Check that it is connected by USB and switched on"
            }
            GP_ERROR_CAMERA_BUSY => "the camera is busy. Try again in a moment",
            _ => {
                // SAFETY: returns a static string for any result code.
                let text = unsafe { (self.result_as_string)(result) };
                if text.is_null() {
                    bail!("libgphoto2 error {result}");
                }
                // SAFETY: a static, NUL-terminated string.
                bail!("{}", unsafe { CStr::from_ptr(text) }.to_string_lossy().to_lowercase());
            }
        };
        bail!("{plain}")
    }
}

fn text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Cameras on USB. Card readers and cameras showing as drives, which
/// libgphoto2 also lists, are left to importing.
pub fn detect() -> Vec<Found> {
    let Ok(gp) = gphoto() else { return Vec::new() };
    // SAFETY: the list and context are made, read and freed here, and the
    // strings read are copied before the list is freed.
    unsafe {
        let context = (gp.context_new)();
        let mut list = std::ptr::null_mut();
        let mut found = Vec::new();
        if (gp.list_new)(&mut list) >= 0 {
            if (gp.camera_autodetect)(list, context) >= 0 {
                for index in 0..(gp.list_count)(list).max(0) {
                    let (mut name, mut port) = (std::ptr::null(), std::ptr::null());
                    if (gp.list_get_name)(list, index, &mut name) < 0
                        || (gp.list_get_value)(list, index, &mut port) < 0
                        || name.is_null()
                        || port.is_null()
                    {
                        continue;
                    }
                    let port = CStr::from_ptr(port).to_string_lossy().into_owned();
                    if port.starts_with("usb:") && port.len() > "usb:".len() {
                        found.push(Found { name: CStr::from_ptr(name).to_string_lossy().into_owned(), port });
                    }
                }
            }
            (gp.list_free)(list);
        }
        if !context.is_null() {
            (gp.context_unref)(context);
        }
        found
    }
}

pub struct GphotoCamera {
    gp: &'static Gphoto,
    camera: *mut GpCamera,
    context: *mut GpContext,
    ports: *mut GpPortInfoList,
}

// SAFETY: libgphoto2 objects may move between threads; the capture thread
// is the only one that uses them once the camera is open.
unsafe impl Send for GphotoCamera {}

impl Drop for GphotoCamera {
    fn drop(&mut self) {
        // SAFETY: each was made by `open` and is released once, here.
        unsafe {
            if !self.camera.is_null() {
                (self.gp.camera_exit)(self.camera, self.context);
                (self.gp.camera_unref)(self.camera);
            }
            if !self.ports.is_null() {
                (self.gp.port_info_list_free)(self.ports);
            }
            if !self.context.is_null() {
                (self.gp.context_unref)(self.context);
            }
        }
    }
}

/// Connects to the camera at `port` ("usb:001,005").
pub fn open(port: &str) -> Result<Box<dyn Camera>> {
    let gp = gphoto()?;
    let path = CString::new(port)?;
    // SAFETY: the camera is filled in step by step; on any failure `Drop`
    // releases what was made.
    unsafe {
        let mut opened = GphotoCamera {
            gp,
            camera: std::ptr::null_mut(),
            context: (gp.context_new)(),
            ports: std::ptr::null_mut(),
        };
        gp.check((gp.port_info_list_new)(&mut opened.ports))?;
        gp.check((gp.port_info_list_load)(opened.ports))?;
        let index = gp.check((gp.port_info_list_lookup_path)(opened.ports, path.as_ptr()))?;
        let mut info: GpPortInfo = std::ptr::null_mut();
        gp.check((gp.port_info_list_get_info)(opened.ports, index, &mut info))?;
        gp.check((gp.camera_new)(&mut opened.camera))?;
        gp.check((gp.camera_set_port_info)(opened.camera, info))?;
        // With the port set and no model, libgphoto2 works out which camera is on it.
        gp.check((gp.camera_init)(opened.camera, opened.context))?;
        Ok(Box::new(opened))
    }
}

impl Camera for GphotoCamera {
    fn trigger(&mut self) -> Result<Shot> {
        // SAFETY: `path` is a buffer of the size libgphoto2 writes.
        let mut path: FilePath = unsafe { std::mem::zeroed() };
        let result = unsafe { (self.gp.camera_capture)(self.camera, GP_CAPTURE_IMAGE, &mut path, self.context) };
        if result == GP_ERROR {
            // What a camera on autofocus says when it can't find focus.
            bail!("it may not have found focus. With film, set the lens to manual focus");
        }
        self.gp.check(result)?;
        Ok(Shot { folder: text(&path.folder), name: text(&path.name) })
    }

    fn wait(&mut self, timeout: Duration) -> Result<Option<Shot>> {
        let mut kind: c_int = 0;
        let mut data: *mut c_void = std::ptr::null_mut();
        let millis = timeout.as_millis().min(c_int::MAX as u128) as c_int;
        // SAFETY: libgphoto2 sets `kind` and `data`; `data` is malloc'd
        // (or null) and freed here once read.
        unsafe {
            self.gp.check((self.gp.camera_wait_for_event)(self.camera, millis, &mut kind, &mut data, self.context))?;
            let shot = (kind == GP_EVENT_FILE_ADDED && !data.is_null()).then(|| {
                let path = &*(data as *const FilePath);
                Shot { folder: text(&path.folder), name: text(&path.name) }
            });
            if !data.is_null() {
                free(data);
            }
            Ok(shot)
        }
    }

    fn download(&mut self, shot: &Shot, to: &Path) -> Result<()> {
        let folder = CString::new(shot.folder.as_str())?;
        let name = CString::new(shot.name.as_str())?;
        // SAFETY: the file is made, filled, read and released here; its data
        // is copied out before it is released.
        unsafe {
            let mut file = std::ptr::null_mut();
            self.gp.check((self.gp.file_new)(&mut file))?;
            let read = (|| {
                self.gp.check((self.gp.camera_file_get)(
                    self.camera,
                    folder.as_ptr(),
                    name.as_ptr(),
                    GP_FILE_TYPE_NORMAL,
                    file,
                    self.context,
                ))?;
                let (mut data, mut size): (*const c_char, c_ulong) = (std::ptr::null(), 0);
                self.gp.check((self.gp.file_get_data_and_size)(file, &mut data, &mut size))?;
                if data.is_null() || size == 0 {
                    bail!("the camera sent an empty file");
                }
                std::fs::write(to, std::slice::from_raw_parts(data as *const u8, size as usize))
                    .with_context(|| format!("writing {}", to.display()))
            })();
            (self.gp.file_unref)(file);
            read
        }
    }
}
