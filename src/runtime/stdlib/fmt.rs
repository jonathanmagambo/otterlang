use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[no_mangle]
pub extern "C" fn otter_std_fmt_println(msg: *const u8) {
    if msg.is_null() {
        println!();
        return;
    }

    unsafe {
        let c_str = std::ffi::CStr::from_ptr(msg as *const i8);
        if let Ok(s) = c_str.to_str() {
            println!("{}", s);
        } else {
            eprintln!("[fmt.println: invalid UTF-8]");
        }
    }
}

#[no_mangle]
pub extern "C" fn otter_std_fmt_print(msg: *const u8) {
    if msg.is_null() {
        return;
    }

    unsafe {
        let c_str = std::ffi::CStr::from_ptr(msg as *const i8);
        if let Ok(s) = c_str.to_str() {
            print!("{}", s);
        } else {
            eprint!("[fmt.print: invalid UTF-8]");
        }
    }
}

#[no_mangle]
pub extern "C" fn otter_std_fmt_eprintln(msg: *const u8) {
    if msg.is_null() {
        eprintln!();
        return;
    }

    unsafe {
        let c_str = std::ffi::CStr::from_ptr(msg as *const i8);
        if let Ok(s) = c_str.to_str() {
            eprintln!("{}", s);
        } else {
            eprintln!("[fmt.eprintln: invalid UTF-8]");
        }
    }
}

fn register_std_fmt_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "fmt.println".into(),
        symbol: "otter_std_fmt_println".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "fmt.print".into(),
        symbol: "otter_std_fmt_print".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "fmt.eprintln".into(),
        symbol: "otter_std_fmt_eprintln".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_fmt_symbols,
    }
}
