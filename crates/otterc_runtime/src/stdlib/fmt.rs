use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

fn register_std_error_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "runtime.push_context".into(),
        symbol: "otter_error_push_context".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.pop_context".into(),
        symbol: "otter_error_pop_context".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.raise".into(),
        symbol: "otter_error_raise".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.clear".into(),
        symbol: "otter_error_clear".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "runtime.has_error".into(),
        symbol: "otter_error_has_error".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "runtime.get_message".into(),
        symbol: "otter_error_get_message".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "runtime.rethrow".into(),
        symbol: "otter_error_rethrow".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "runtime",
        autoload: false,
        register: register_std_error_symbols,
    }
}
