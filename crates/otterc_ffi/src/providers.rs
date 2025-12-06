use once_cell::sync::OnceCell;

use otterc_symbol::registry::SymbolRegistry;

#[derive(Clone, Copy)]
pub struct SymbolProvider {
    pub namespace: &'static str,
    pub autoload: bool,
    pub register: fn(&SymbolRegistry),
}

inventory::collect!(crate::providers::SymbolProvider);

fn register_builtin_symbols(registry: &SymbolRegistry) {
    for provider in inventory::iter::<SymbolProvider> {
        if provider.autoload {
            registry.mark_module_active(provider.namespace);
            (provider.register)(registry);
        } else {
            registry.register_lazy_module(provider.namespace, provider.register);
        }
    }
}

static STD_INIT: OnceCell<()> = OnceCell::new();

pub fn bootstrap_stdlib() -> &'static SymbolRegistry {
    let registry = SymbolRegistry::global();
    STD_INIT.get_or_init(|| {
        register_builtin_symbols(registry);
    });
    registry
}
