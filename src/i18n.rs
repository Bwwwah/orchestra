//! French or English. Every text of the interface is written in both, side by side, with
//! `tr!("français", "English")` (or `trf!` for a `format!`): the compiler checks both, and
//! changing the language takes effect at the next frame.
//!
//! The choice is in the settings: "auto" (the system's language: English unless it is
//! French), "fr" or "en".

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

/// The interface is in English.
pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// Apply a setting: "auto", "fr" or "en".
pub fn set(choice: &str) {
    let en = match choice {
        "fr" => false,
        "en" => true,
        _ => system_is_english(),
    };
    EN.store(en, Ordering::Relaxed);
}

/// The system's language, as "fr-FR", "en-US"…
pub fn system_locale() -> String {
    std::env::var("ORCHESTRA_LANG").ok().filter(|s| !s.is_empty()).or_else(sys_locale::get_locale).unwrap_or_else(|| "fr".into())
}

/// English for every language but French (the app speaks these two).
pub fn system_is_english() -> bool {
    !system_locale().to_lowercase().starts_with("fr")
}

#[macro_export]
macro_rules! tr {
    ($fr:literal, $en:literal) => {
        if $crate::i18n::en() { $en } else { $fr }
    };
}

#[macro_export]
macro_rules! trf {
    ($fr:literal, $en:literal $($rest:tt)*) => {
        if $crate::i18n::en() { format!($en $($rest)*) } else { format!($fr $($rest)*) }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn french_by_default() {
        let n = 3;
        assert_eq!(crate::tr!("Projets", "Projects"), "Projets");
        assert_eq!(crate::trf!("{n} pages", "{n} pages!"), "3 pages");
    }
}
