#![allow(unsafe_code)]

use diode::aux::{self, file};
use std::{
    ffi::{CStr, c_char},
    net::SocketAddr,
    path::PathBuf,
    ptr,
    str::FromStr,
};

/// Opaque C handle; internal layout is not exposed to foreign callers (ANSSI R44/R51).
#[repr(C)]
pub struct DiodeFileConfig {
    _private: [u8; 0],
}

type ConfigInner = file::Config<aux::DiodeSend>;

fn config_from_handle(ptr: *mut DiodeFileConfig) -> Option<&'static mut ConfigInner> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: pointer originates from `diode_new_config` or is null-checked.
        Some(unsafe { &mut *(ptr.cast::<ConfigInner>()) })
    }
}

/// # Safety
///
/// `ptr_addr` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_new_config(
    ptr_addr: *const c_char,
    buffer_size: u32,
) -> *mut DiodeFileConfig {
    if ptr_addr.is_null() {
        return ptr::null_mut();
    }
    let cstr_addr = unsafe { CStr::from_ptr(ptr_addr) };
    let rust_addr = match std::str::from_utf8(cstr_addr.to_bytes()) {
        Ok(addr) => addr,
        Err(_) => return ptr::null_mut(),
    };
    let Ok(socket_addr) = SocketAddr::from_str(rust_addr) else {
        return ptr::null_mut();
    };

    let Ok(buffer_size) = usize::try_from(buffer_size) else {
        return ptr::null_mut();
    };
    if buffer_size == 0 {
        return ptr::null_mut();
    }

    let config = Box::new(file::Config {
        diode: aux::DiodeSend::Tcp(socket_addr),
        buffer_size,
        hash: false,
        max_files: 0,
    });
    Box::into_raw(config).cast::<DiodeFileConfig>()
}

/// # Safety
///
/// `ptr` must be a valid handle returned by [`diode_new_config`], or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_free_config(ptr: *mut DiodeFileConfig) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr.cast::<ConfigInner>()));
    }
}

/// # Safety
///
/// `ptr` must be a valid handle returned by [`diode_new_config`], or null.
/// `ptr_filepath` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_send_file(
    ptr: *mut DiodeFileConfig,
    ptr_filepath: *const c_char,
) -> u32 {
    let Some(config) = config_from_handle(ptr) else {
        return 0;
    };

    if ptr_filepath.is_null() {
        return 0;
    }
    let cstr_filepath = unsafe { CStr::from_ptr(ptr_filepath) };
    let rust_filepath = match std::str::from_utf8(cstr_filepath.to_bytes()) {
        Ok(path) => path.to_string(),
        Err(_) => return 0,
    };

    match file::send::send_file(config, &rust_filepath) {
        Ok(result) => u32::try_from(result).unwrap_or(0),
        Err(_) => 0,
    }
}

/// # Safety
///
/// `ptr` must be a valid handle returned by [`diode_new_config`], or null.
/// `ptr_odir` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_receive_files(
    ptr: *mut DiodeFileConfig,
    ptr_odir: *const c_char,
) -> u32 {
    let Some(config) = config_from_handle(ptr) else {
        return 0;
    };
    let aux::DiodeSend::Tcp(socket_addr) = config.diode else {
        return 0;
    };

    let config = file::Config {
        diode: aux::DiodeReceive {
            from_tcp: Some(socket_addr),
            from_unix: None,
        },
        buffer_size: config.buffer_size,
        hash: false,
        max_files: 0,
    };

    if ptr_odir.is_null() {
        return 0;
    }
    let cstr_odir = unsafe { CStr::from_ptr(ptr_odir) };
    let rust_odir = match std::str::from_utf8(cstr_odir.to_bytes()) {
        Ok(path) => path.to_string(),
        Err(_) => return 0,
    };
    let odir = PathBuf::from(rust_odir);

    match file::receive::receive_files(&config, &odir) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn invalid_address_returns_null_opaque_handle() {
        let bad = CString::new("not-an-address").expect("cstring");
        let ptr = unsafe { diode_new_config(bad.as_ptr(), 4096) };
        assert!(ptr.is_null());
        unsafe { diode_free_config(ptr) };
    }
}
