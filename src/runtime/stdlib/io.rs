use std::ffi::{CStr, CString};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

// ============================================================================
// Buffer Management
// ============================================================================

type HandleId = u64;
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn next_handle_id() -> HandleId {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)
}

struct Buffer {
    _id: HandleId,
    data: Vec<u8>,
    position: usize,
}

static BUFFERS: Lazy<RwLock<std::collections::HashMap<HandleId, Buffer>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

// ============================================================================
// File I/O Functions
// ============================================================================

/// writes a message `message` to stdout
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_print(message: *const c_char) {
    if message.is_null() {
        return;
    }

    unsafe {
        if let Ok(str_ref) = CStr::from_ptr(message).to_str() {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(str_ref.as_bytes());
            let _ = stdout.flush();
        }
    }
}

/// writes a message `message` to stdout with a newline
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
#[expect(
    clippy::print_stdout,
    reason = "We want to print to stdout with println"
)]
pub unsafe extern "C" fn otter_std_io_println(message: *const c_char) {
    if message.is_null() {
        println!();
        return;
    }

    unsafe {
        if let Ok(str_ref) = CStr::from_ptr(message).to_str() {
            println!("{str_ref}");
        }
    }
}

/// writes a message `message` to stderr with a newline
///
/// # Safety
///
/// this function dereferences a raw pointer
#[expect(
    clippy::print_stderr,
    reason = "We want to print to stderr with eprintln"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_eprintln(message: *const c_char) {
    unsafe {
        if message.is_null() {
            eprintln!();
            return;
        }

        let Ok(str_ref) = CStr::from_ptr(message).to_str() else {
            eprintln!("[io.eprintln: invalid UTF-8]");
            return;
        };

        eprintln!("{str_ref}");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_io_read_line() -> *mut c_char {
    let mut line = String::new();
    let mut stdin = io::stdin().lock();
    match stdin.read_line(&mut line) {
        Ok(0) | Err(_) => std::ptr::null_mut(),
        Ok(_) => {
            let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
            CString::new(trimmed)
                .map(CString::into_raw)
                .unwrap_or_else(|_| std::ptr::null_mut())
        }
    }
}

/// attempts to free the string pointed to by `ptr`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

/// attempts to read and return the contents of the file pointed to by `path`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_read(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };

    match fs::read_to_string(&path_str) {
        Ok(content) => CString::new(content)
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        Err(_) => std::ptr::null_mut(),
    }
}

/// attempts to write `data` to the file pointed to by `path`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_write(path: *const c_char, data: *const c_char) -> i32 {
    if path.is_null() || data.is_null() {
        return 0;
    }

    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };

    let data_str = unsafe { CStr::from_ptr(data).to_str().unwrap_or("").to_string() };

    match fs::write(&path_str, data_str.as_bytes()) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

///
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_copy(src: *const c_char, dst: *const c_char) -> i32 {
    if src.is_null() || dst.is_null() {
        return 0;
    }

    let src_str = unsafe { CStr::from_ptr(src).to_str().unwrap_or("").to_string() };

    let dst_str = unsafe { CStr::from_ptr(dst).to_str().unwrap_or("").to_string() };

    match fs::copy(&src_str, &dst_str) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

///
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_lines(path: *const c_char) -> u64 {
    if path.is_null() {
        return 0;
    }

    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };

    match fs::File::open(&path_str) {
        Ok(file) => {
            let reader = BufReader::new(file);
            let mut lines_list = Vec::new();

            for line_str in reader.lines().map_while(Result::ok) {
                lines_list.push(line_str);
            }

            // Create a list handle using builtins module
            // We'll use the builtin list functions directly
            unsafe extern "C" {
                fn otter_builtin_list_new() -> u64;
                fn otter_builtin_append_list_string(handle: u64, val: *const c_char) -> i32;
            }

            let list_handle = unsafe { otter_builtin_list_new() };

            for line in lines_list {
                let cstr = CString::new(line).unwrap();
                let ptr = cstr.into_raw();
                unsafe {
                    let _ = otter_builtin_append_list_string(list_handle, ptr);
                    let _ = CString::from_raw(ptr);
                }
            }

            list_handle
        }
        Err(_) => 0,
    }
}

// ============================================================================
// Buffer Operations
// ============================================================================

/// allocates a new vector and returns a handle to it
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_buffer(data: *const c_char) -> u64 {
    let id = next_handle_id();
    let buffer = if data.is_null() {
        Buffer {
            _id: id,
            data: Vec::new(),
            position: 0,
        }
    } else {
        let data_str = unsafe { CStr::from_ptr(data).to_str().unwrap_or("").to_string() };
        Buffer {
            _id: id,
            data: data_str.into_bytes(),
            position: 0,
        }
    };

    BUFFERS.write().insert(id, buffer);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_io_buffer_read(handle: u64, n: i64) -> *mut c_char {
    let mut buffers = BUFFERS.write();
    if let Some(buffer) = buffers.get_mut(&handle) {
        let remaining = buffer.data.len() - buffer.position;
        let read_size = if n <= 0 || n as usize > remaining {
            remaining
        } else {
            n as usize
        };

        if read_size == 0 {
            return std::ptr::null_mut();
        }

        let start = buffer.position;
        let end = buffer.position + read_size;
        buffer.position = end;

        let bytes = &buffer.data[start..end];
        if let Ok(s) = String::from_utf8(bytes.to_vec()) {
            CString::new(s)
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    } else {
        std::ptr::null_mut()
    }
}

/// attempts to write the bytes stored in `bytes` to the buffer pointed to by `handle`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_buffer_write(handle: u64, bytes: *const c_char) -> i32 {
    if bytes.is_null() {
        return 0;
    }

    let bytes_str = unsafe { CStr::from_ptr(bytes).to_str().unwrap_or("").to_string() };

    let mut buffers = BUFFERS.write();
    if let Some(buffer) = buffers.get_mut(&handle) {
        buffer.data.extend_from_slice(bytes_str.as_bytes());
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_io_buffer_clear(handle: u64) {
    let mut buffers = BUFFERS.write();
    if let Some(buffer) = buffers.get_mut(&handle) {
        buffer.data.clear();
        buffer.position = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_io_buffer_data(handle: u64) -> *mut c_char {
    let buffers = BUFFERS.read();
    if let Some(buffer) = buffers.get(&handle) {
        if let Ok(s) = String::from_utf8(buffer.data.clone()) {
            CString::new(s)
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    } else {
        std::ptr::null_mut()
    }
}

// ============================================================================
// File System Operations
// ============================================================================

/// returns whether or not a directory exists
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_exists(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    if fs::metadata(&path_str).is_ok() {
        1
    } else {
        0
    }
}

/// attempts to create the directory pointed to by `path`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_mkdir(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    match fs::create_dir_all(&path_str) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// attempts to remove the directory pointed to by `path`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_rmdir(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    match fs::remove_dir_all(&path_str) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

///
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_remove(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    match fs::remove_file(&path_str) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

/// lists all items under directory `path`, returning a handle to a list
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_list_dir(path: *const c_char) -> u64 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };

    match fs::read_dir(&path_str) {
        Ok(entries) => {
            unsafe extern "C" {
                fn otter_builtin_list_new() -> u64;
                fn otter_builtin_append_list_string(handle: u64, val: *const c_char) -> i32;
            }

            let list_handle = unsafe { otter_builtin_list_new() };

            for entry in entries {
                if let Ok(entry) = entry
                    && let Some(file_name) = entry.file_name().to_str()
                {
                    let cstr = CString::new(file_name).unwrap();
                    let ptr = cstr.into_raw();
                    unsafe {
                        let _ = otter_builtin_append_list_string(list_handle, ptr);
                        let _ = CString::from_raw(ptr);
                    }
                }
            }

            list_handle
        }
        Err(_) => 0,
    }
}

/// retrieve whether or not the path pointed to by `path` is a file
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_is_file(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    if let Ok(metadata) = fs::metadata(&path_str) {
        if metadata.is_file() { 1 } else { 0 }
    } else {
        0
    }
}

/// retrieve whether or not the path pointed to by `path` is a directory
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_is_dir(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    if let Ok(metadata) = fs::metadata(&path_str) {
        if metadata.is_dir() { 1 } else { 0 }
    } else {
        0
    }
}

/// retrieve the size of a given file
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_io_file_size(path: *const c_char) -> i64 {
    if path.is_null() {
        return -1;
    }
    let path_str = unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() };
    if let Ok(metadata) = fs::metadata(&path_str) {
        metadata.len() as i64
    } else {
        -1
    }
}

fn register_io_prelude_symbols(registry: &SymbolRegistry) {
    let sig = FfiSignature::new(vec![FfiType::Str], FfiType::Unit);
    registry.register(FfiFunction {
        name: "print".into(),
        symbol: "otter_std_io_print".into(),
        signature: sig.clone(),
    });
    registry.register(FfiFunction {
        name: "println".into(),
        symbol: "otter_std_io_println".into(),
        signature: sig,
    });
}

fn register_std_io_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "std.io.print".into(),
        symbol: "otter_std_io_print".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "std.io.println".into(),
        symbol: "otter_std_io_println".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "std.io.eprintln".into(),
        symbol: "otter_std_io_eprintln".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "io.eprintln".into(),
        symbol: "otter_std_io_eprintln".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "eprintln".into(),
        symbol: "otter_std_io_eprintln".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "std.io.read_line".into(),
        symbol: "otter_std_io_read_line".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.io.free".into(),
        symbol: "otter_std_io_free_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "io.read".into(),
        symbol: "otter_std_io_read".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "io.write".into(),
        symbol: "otter_std_io_write".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "io.copy".into(),
        symbol: "otter_std_io_copy".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "io.lines".into(),
        symbol: "otter_std_io_lines".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "io.buffer".into(),
        symbol: "otter_std_io_buffer".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "io.buffer.read".into(),
        symbol: "otter_std_io_buffer_read".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "io.buffer.write".into(),
        symbol: "otter_std_io_buffer_write".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "io.buffer.clear".into(),
        symbol: "otter_std_io_buffer_clear".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "io.buffer.data".into(),
        symbol: "otter_std_io_buffer_data".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "fs.exists".into(),
        symbol: "otter_std_io_exists".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.mkdir".into(),
        symbol: "otter_std_io_mkdir".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.rmdir".into(),
        symbol: "otter_std_io_rmdir".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.remove".into(),
        symbol: "otter_std_io_remove".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.list_dir".into(),
        symbol: "otter_std_io_list_dir".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "fs.is_file".into(),
        symbol: "otter_std_io_is_file".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.is_dir".into(),
        symbol: "otter_std_io_is_dir".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "fs.file_size".into(),
        symbol: "otter_std_io_file_size".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I64),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "io_prelude",
        autoload: true,
        register: register_io_prelude_symbols,
    }
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "io",
        autoload: false,
        register: register_std_io_symbols,
    }
}
