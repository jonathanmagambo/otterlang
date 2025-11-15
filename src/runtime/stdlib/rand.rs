use std::ffi::CString;
use std::os::raw::c_char;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

// ============================================================================
// Random Number Generator
// Using a simple LCG-based PRNG for deterministic seeding
// ============================================================================

struct RngState {
    seed: u64,
}

static RNG_STATE: Lazy<Mutex<RngState>> = Lazy::new(|| {
    Mutex::new(RngState {
        seed: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
    })
});

// Linear Congruential Generator
fn lcg_next(seed: &mut u64) -> u64 {
    // LCG parameters (from Numerical Recipes)
    *seed = seed.wrapping_mul(1664525u64).wrapping_add(1013904223u64);
    *seed
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_rand_seed(n: i64) {
    let mut state = RNG_STATE.lock();
    state.seed = n as u64;
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_rand_int(min: i64, max: i64) -> i64 {
    let mut state = RNG_STATE.lock();
    let next = lcg_next(&mut state.seed);

    if min >= max {
        return min;
    }

    let range = (max - min) as u64;
    
    min + (next % range) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_rand_float() -> f64 {
    let mut state = RNG_STATE.lock();
    let next = lcg_next(&mut state.seed);

    // Convert to float in [0.0, 1.0)
    (next as f64) / (u64::MAX as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_rand_bytes(n: i64) -> *mut c_char {
    if n <= 0 || n > 1024 {
        return std::ptr::null_mut();
    }

    let mut state = RNG_STATE.lock();
    let mut bytes = Vec::with_capacity(n as usize);

    for _ in 0..n {
        let next = lcg_next(&mut state.seed);
        bytes.push((next & 0xFF) as u8);
    }

    // Convert bytes to hex string
    let hex_string: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();

    CString::new(hex_string)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_rand_uuid() -> *mut c_char {
    let mut state = RNG_STATE.lock();

    // Generate UUID v4 format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
    let mut uuid_bytes = Vec::new();
    for _ in 0..16 {
        let next = lcg_next(&mut state.seed);
        uuid_bytes.push((next & 0xFF) as u8);
    }

    // Set version (4) and variant bits
    uuid_bytes[6] = (uuid_bytes[6] & 0x0F) | 0x40; // Version 4
    uuid_bytes[8] = (uuid_bytes[8] & 0x3F) | 0x80; // Variant 10

    // Format as UUID string
    let uuid_str = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid_bytes[0],
        uuid_bytes[1],
        uuid_bytes[2],
        uuid_bytes[3],
        uuid_bytes[4],
        uuid_bytes[5],
        uuid_bytes[6],
        uuid_bytes[7],
        uuid_bytes[8],
        uuid_bytes[9],
        uuid_bytes[10],
        uuid_bytes[11],
        uuid_bytes[12],
        uuid_bytes[13],
        uuid_bytes[14],
        uuid_bytes[15]
    );

    CString::new(uuid_str)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn register_std_rand_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "rand.seed".into(),
        symbol: "otter_std_rand_seed".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "rand.int".into(),
        symbol: "otter_std_rand_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "rand.float".into(),
        symbol: "otter_std_rand_float".into(),
        signature: FfiSignature::new(vec![], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "rand.bytes".into(),
        symbol: "otter_std_rand_bytes".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "rand.uuid".into(),
        symbol: "otter_std_rand_uuid".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_rand_symbols,
    }
}
