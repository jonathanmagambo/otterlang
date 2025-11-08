use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[no_mangle]
pub extern "C" fn otter_std_math_abs(value: f64) -> f64 {
    libm::fabs(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_sqrt(value: f64) -> f64 {
    libm::sqrt(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_pow(base: f64, exponent: f64) -> f64 {
    libm::pow(base, exponent)
}

#[no_mangle]
pub extern "C" fn otter_std_math_exp(value: f64) -> f64 {
    libm::exp(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_log(value: f64) -> f64 {
    libm::log(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_sin(value: f64) -> f64 {
    libm::sin(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_cos(value: f64) -> f64 {
    libm::cos(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_tan(value: f64) -> f64 {
    libm::tan(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

#[no_mangle]
pub extern "C" fn otter_std_math_floor(value: f64) -> f64 {
    libm::floor(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_ceil(value: f64) -> f64 {
    libm::ceil(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_round(value: f64) -> f64 {
    libm::round(value)
}

#[no_mangle]
pub extern "C" fn otter_std_math_clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[no_mangle]
pub extern "C" fn otter_std_math_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

#[no_mangle]
pub extern "C" fn otter_std_math_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

#[no_mangle]
pub extern "C" fn otter_std_math_hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

#[no_mangle]
pub extern "C" fn otter_std_math_lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[no_mangle]
pub extern "C" fn otter_std_math_randf() -> f64 {
    // Use simple pseudo-random (for deterministic, use rand module)
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as f64;
    (nanos % 1000000.0) / 1000000.0
}

#[no_mangle]
pub extern "C" fn otter_std_math_randi(max: i64) -> i64 {
    if max <= 0 {
        return 0;
    }
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as i64;
    nanos % max
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_math_mean(arr: *const f64, len: i64) -> f64 {
    if arr.is_null() || len <= 0 {
        return 0.0;
    }

    unsafe {
        let slice = std::slice::from_raw_parts(arr, len as usize);
        let sum: f64 = slice.iter().sum();
        sum / (len as f64)
    }
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_math_std(arr: *const f64, len: i64) -> f64 {
    if arr.is_null() || len <= 0 {
        return 0.0;
    }

    unsafe {
        let slice = std::slice::from_raw_parts(arr, len as usize);
        let mean: f64 = slice.iter().sum::<f64>() / (len as f64);
        let variance: f64 = slice.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (len as f64);
        libm::sqrt(variance)
    }
}

#[no_mangle]
pub unsafe extern "C" fn otter_std_math_sum(arr: *const f64, len: i64) -> f64 {
    if arr.is_null() || len <= 0 {
        return 0.0;
    }

    unsafe {
        let slice = std::slice::from_raw_parts(arr, len as usize);
        slice.iter().sum()
    }
}

fn register_std_math_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "math.abs".into(),
        symbol: "otter_std_math_abs".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.sqrt".into(),
        symbol: "otter_std_math_sqrt".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.pow".into(),
        symbol: "otter_std_math_pow".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.exp".into(),
        symbol: "otter_std_math_exp".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.log".into(),
        symbol: "otter_std_math_log".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.sin".into(),
        symbol: "otter_std_math_sin".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.cos".into(),
        symbol: "otter_std_math_cos".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.tan".into(),
        symbol: "otter_std_math_tan".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.atan2".into(),
        symbol: "otter_std_math_atan2".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.floor".into(),
        symbol: "otter_std_math_floor".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.ceil".into(),
        symbol: "otter_std_math_ceil".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.round".into(),
        symbol: "otter_std_math_round".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.clamp".into(),
        symbol: "otter_std_math_clamp".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.min".into(),
        symbol: "otter_std_math_min".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.max".into(),
        symbol: "otter_std_math_max".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.hypot".into(),
        symbol: "otter_std_math_hypot".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.lerp".into(),
        symbol: "otter_std_math_lerp".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64, FfiType::F64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.randf".into(),
        symbol: "otter_std_math_randf".into(),
        signature: FfiSignature::new(vec![], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.randi".into(),
        symbol: "otter_std_math_randi".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "math.mean".into(),
        symbol: "otter_std_math_mean".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.std".into(),
        symbol: "otter_std_math_std".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "math.sum".into(),
        symbol: "otter_std_math_sum".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::F64),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_math_symbols,
    }
}
