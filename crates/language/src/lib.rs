#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageFeatureFlags {
    pub result_option_core: bool,
    pub match_exhaustiveness: bool,
    pub newtype_aliases: bool,
}

impl LanguageFeatureFlags {
    pub const RESULT_OPTION_CORE: &'static str = "result_option_core";
    pub const MATCH_EXHAUSTIVENESS: &'static str = "match_exhaustiveness";
    pub const NEWTYPE_ALIASES: &'static str = "newtype_aliases";

    pub fn enable(&mut self, feature: &str) -> bool {
        match feature {
            Self::RESULT_OPTION_CORE => {
                self.result_option_core = true;
                true
            }
            Self::MATCH_EXHAUSTIVENESS => {
                self.match_exhaustiveness = true;
                true
            }
            Self::NEWTYPE_ALIASES => {
                self.newtype_aliases = true;
                true
            }
            _ => false,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.result_option_core || self.match_exhaustiveness || self.newtype_aliases
    }
}
