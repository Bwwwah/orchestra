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
    std::env::var("ORCHESTRA_LANG")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(sys_locale::get_locale)
        .unwrap_or_else(|| "fr".into())
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

/// The plural mark for `n` things: "s" or "". In French 0 and 1 take the singular
/// (« 0 projet », « 1 projet », « 2 projets »), in English only 1 does. Used as
/// `trf!("{n} projet{s}", "{n} project{s}", s = plural(n))`, instead of « projet(s) ».
pub fn plural(n: usize) -> &'static str {
    plural_in(n, en())
}

fn plural_in(n: usize, en: bool) -> &'static str {
    if (en && n != 1) || (!en && n > 1) {
        "s"
    } else {
        ""
    }
}

/// A number with one decimal, with a decimal comma in French (« 14,3 ») and a point in
/// English (« 14.3 »).
pub fn dec1(x: f32) -> String {
    dec1_in(x, en())
}

fn dec1_in(x: f32, en: bool) -> String {
    let s = format!("{x:.1}");
    if en { s } else { s.replace('.', ",") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn french_by_default() {
        let n = 3;
        assert_eq!(crate::tr!("Projets", "Projects"), "Projets");
        assert_eq!(crate::trf!("{n} pages", "{n} pages!"), "3 pages");
    }

    #[test]
    fn plurals_and_decimals() {
        use super::{dec1_in, plural_in};
        assert_eq!([0, 1, 2].map(|n| plural_in(n, false)), ["", "", "s"]);
        assert_eq!([0, 1, 2].map(|n| plural_in(n, true)), ["s", "", "s"]);
        assert_eq!(dec1_in(14.2857, false), "14,3");
        assert_eq!(dec1_in(14.2857, true), "14.3");
        let s = super::plural(3);
        assert_eq!(
            crate::trf!("{} projet{s} prêt{s}", "{} project{s} ready", 3),
            "3 projets prêts"
        );
    }
}
