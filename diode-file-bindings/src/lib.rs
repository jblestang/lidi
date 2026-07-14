#![allow(unsafe_code)]

use diode::aux::{self, file};
use std::{
    ffi::{CStr, c_char},
    net::SocketAddr,
    path::PathBuf,
    ptr,
    str::FromStr,
};

/// # Safety
///
/// `ptr_addr` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_new_config(
    ptr_addr: *const c_char,
    buffer_size: u32,
) -> *mut file::Config<aux::DiodeSend> {
    if ptr_addr.is_null() {
        return ptr::null_mut();
    }
    let cstr_addr = unsafe { CStr::from_ptr(ptr_addr) };
    let rust_addr = String::from_utf8_lossy(cstr_addr.to_bytes()).to_string();
    let Ok(socket_addr) = SocketAddr::from_str(&rust_addr) else {
        return ptr::null_mut();
    };

    let config = Box::new(file::Config {
        diode: aux::DiodeSend::Tcp(socket_addr),
        buffer_size: buffer_size as usize,
        hash: false,
        max_files: 0,
    });
    Box::into_raw(config)
}

/// # Safety
///
/// `ptr` must be a valid config pointer returned by [`diode_new_config`], or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_free_config(ptr: *mut file::Config<aux::DiodeSend>) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr));
    }
}

/// # Safety
///
/// `ptr` must be a valid config pointer returned by [`diode_new_config`], or null.
/// `ptr_filepath` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_send_file(
    ptr: *mut file::Config<aux::DiodeSend>,
    ptr_filepath: *const c_char,
) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let Some(config) = (unsafe { ptr.as_ref() }) else {
        return 0;
    };

    if ptr_filepath.is_null() {
        return 0;
    }
    let cstr_filepath = unsafe { CStr::from_ptr(ptr_filepath) };
    let rust_filepath = String::from_utf8_lossy(cstr_filepath.to_bytes()).to_string();

    match file::send::send_file(config, &rust_filepath) {
        Ok(result) => u32::try_from(result).unwrap_or(0),
        Err(_) => 0,
    }
}

/// # Safety
///
/// `ptr` must be a valid config pointer returned by [`diode_new_config`], or null.
/// `ptr_odir` must be a valid null-terminated C string when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn diode_receive_files(
    ptr: *mut file::Config<aux::DiodeSend>,
    ptr_odir: *const c_char,
) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let Some(config) = (unsafe { ptr.as_ref() }) else {
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
    let rust_odir = String::from_utf8_lossy(cstr_odir.to_bytes()).to_string();
    let odir = PathBuf::from(rust_odir);

    match file::receive::receive_files(&config, &odir) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}
