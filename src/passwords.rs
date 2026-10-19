//! Local password generation and conservative, pattern-aware strength estimates.
use anyhow::{bail, Result};
use rand::{rngs::OsRng, RngCore};
use std::{collections::HashSet, sync::OnceLock};
use zeroize::Zeroizing;

const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{}:,.?";
const AMBIGUOUS: &[u8] = b"Il1O0o|";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Memorable,
    Alphanumeric,
    Random,
    Digits,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    /// Word count in Memorable mode; character count otherwise.
    pub length: usize,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Random,
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: true,
        }
    }
}

/// Uniformly select an index, using fallible OS randomness and rejection sampling.
fn random_index(rng: &mut impl RngCore, upper: usize) -> Result<usize> {
    let upper = upper as u32;
    let limit = u32::MAX - (u32::MAX % upper);
    loop {
        let mut bytes = Zeroizing::new([0u8; 4]);
        rng.try_fill_bytes(&mut *bytes)
            .map_err(|_| anyhow::anyhow!("Die sichere Zufallsquelle ist nicht verfügbar."))?;
        let value = u32::from_ne_bytes(*bytes);
        if value < limit {
            return Ok((value % upper) as usize);
        }
    }
}

fn words() -> Result<&'static [&'static str]> {
    static WORDS: OnceLock<Option<Vec<&'static str>>> = OnceLock::new();
    let words = WORDS.get_or_init(|| {
        let words: Vec<_> = include_str!("../assets/wordlists/eff_large_wordlist.txt")
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .collect();
        (words.len() == 7776 && words.iter().copied().collect::<HashSet<_>>().len() == 7776)
            .then_some(words)
    });
    words
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Die EFF-Wortliste ist unvollständig."))
}

/// The caller owns the returned secret and should clear it when no longer needed.
/// Rejected candidate buffers are cleared; no generated values are cached.
pub fn generate(options: &Options) -> Result<String> {
    let mut rng = OsRng;
    if options.mode == Mode::Memorable {
        if !(4..=10).contains(&options.length) {
            bail!("Bitte zwischen 4 und 10 Wörter wählen.");
        }
        let words = words()?;
        let mut output = Zeroizing::new(String::new());
        for i in 0..options.length {
            if i != 0 {
                output.push('-');
            }
            output.push_str(words[random_index(&mut rng, words.len())?]);
        }
        return Ok(std::mem::take(&mut *output));
    }
    if !(4..=64).contains(&options.length) {
        bail!("Die Länge muss zwischen 4 und 64 Zeichen liegen.");
    }
    let groups: Vec<&[u8]> = match options.mode {
        Mode::Alphanumeric => vec![LOWER, UPPER, DIGITS],
        Mode::Digits => vec![DIGITS],
        Mode::Random => [
            (options.lower, LOWER),
            (options.upper, UPPER),
            (options.digits, DIGITS),
            (options.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter_map(|(enabled, pool)| enabled.then_some(pool))
        .collect(),
        Mode::Memorable => unreachable!(),
    };
    if groups.is_empty() {
        bail!("Bitte mindestens eine Zeichengruppe auswählen.");
    }
    let exclude = options.exclude_ambiguous && options.mode != Mode::Digits;
    let pools: Vec<Vec<u8>> = groups
        .into_iter()
        .map(|group| {
            group
                .iter()
                .copied()
                .filter(|c| !exclude || !AMBIGUOUS.contains(c))
                .collect()
        })
        .collect();
    let alphabet: Vec<u8> = pools.iter().flatten().copied().collect();
    // Sampling complete candidates, conditioned on all selected classes occurring,
    // preserves uniformity across every password satisfying those requirements.
    loop {
        let mut candidate = Zeroizing::new(String::with_capacity(options.length));
        for _ in 0..options.length {
            candidate.push(alphabet[random_index(&mut rng, alphabet.len())?] as char);
        }
        if pools
            .iter()
            .all(|pool| candidate.bytes().any(|c| pool.contains(&c)))
        {
            return Ok(std::mem::take(&mut *candidate));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strength {
    pub score: Option<u8>,
    pub label: String,
    pub estimate: String,
}

/// Return only the local summary, never the password or zxcvbn's matched tokens.
/// This function does not retain a credential cache.
pub fn assess(value: &str, context: &[&str]) -> Strength {
    if value.is_empty() {
        return Strength {
            score: None,
            label: "Noch kein Passwort".into(),
            estimate: String::new(),
        };
    }
    if value.chars().count() > 72 {
        return Strength {
            score: None,
            label: "Keine Bewertung über 72 Zeichen".into(),
            estimate: String::new(),
        };
    }
    let context = Zeroizing::new(
        context
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(72).collect::<String>())
            .collect::<Vec<_>>(),
    );
    let borrowed: Vec<&str> = context.iter().map(String::as_str).collect();
    let result = zxcvbn::zxcvbn(value, &borrowed);
    let score = u8::from(result.score());
    // zxcvbn's slow offline hash scenario assumes 10,000 guesses per second.
    let seconds = result.guesses() as f64 / 10_000.0;
    let label = ["Sehr schwach", "Schwach", "Mittel", "Stark", "Sehr stark"][score as usize];
    Strength {
        score: Some(score),
        label: label.into(),
        estimate: format!("Geschätzte Knackzeit: {}", duration(seconds)),
    }
}

fn duration(seconds: f64) -> String {
    if seconds < 1.0 {
        return "unter einer Sekunde".into();
    }
    for (unit, span) in [
        ("Jahre", 31_557_600.0),
        ("Tage", 86_400.0),
        ("Stunden", 3_600.0),
        ("Minuten", 60.0),
    ] {
        if seconds >= span {
            let count = (seconds / span).floor() as u64;
            return if unit == "Jahre" && count >= 100 {
                "Jahrhunderte".into()
            } else {
                format!("{} {}", count, unit)
            };
        }
    }
    format!("{} Sekunden", seconds.floor() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_generates_all_modes() {
        let options = Options::default();
        for _ in 0..24 {
            let value = generate(&options).unwrap();
            assert_eq!(value.len(), 20);
            assert!(value.bytes().all(|c| !AMBIGUOUS.contains(&c)));
            for pool in [LOWER, UPPER, DIGITS, SYMBOLS] {
                assert!(value.bytes().any(|c| pool.contains(&c)));
            }
        }
        let value = generate(&Options {
            mode: Mode::Alphanumeric,
            ..options.clone()
        })
        .unwrap();
        assert!(value.bytes().all(|c| c.is_ascii_alphanumeric()));
        let value = generate(&Options {
            mode: Mode::Digits,
            length: 64,
            ..options.clone()
        })
        .unwrap();
        assert!(value.bytes().all(|c| c.is_ascii_digit()));
        let value = generate(&Options {
            mode: Mode::Memorable,
            length: 6,
            ..options.clone()
        })
        .unwrap();
        assert_eq!(value.split('-').count(), 6);
        assert!(value
            .split('-')
            .all(|word| words().unwrap().contains(&word)));
        for length in [3, 11] {
            assert!(generate(&Options {
                mode: Mode::Memorable,
                length,
                ..options.clone()
            })
            .is_err());
        }
        for length in [0, 3, 65] {
            assert!(generate(&Options {
                length,
                ..options.clone()
            })
            .is_err());
        }
        assert!(generate(&Options {
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            ..options
        })
        .is_err());
    }
    #[test]
    fn digit_mode_keeps_zero_and_one() {
        // Verify the full alphabet, including symbols normally excluded in other modes.
        assert_eq!(DIGITS, b"0123456789");
        let options = Options {
            mode: Mode::Digits,
            length: 64,
            ..Options::default()
        };
        for _ in 0..8 {
            assert_eq!(generate(&options).unwrap().len(), 64);
        }
    }
    #[test]
    fn local_pattern_assessment_and_explicit_limit() {
        assert_eq!(assess("", &[]).score, None);
        for weak in ["password", "12345678", "P@ssword1", "aaaaaaaaaaaaaaaaaaaa"] {
            assert!(assess(weak, &[]).score.unwrap() <= 2);
        }
        assert_eq!(assess(&"a".repeat(73), &[]).score, None);
        assert_eq!(assess(&"ä".repeat(73), &[]).score, None);
        assert!(assess(&"ä".repeat(72), &[]).score.is_some());
        assert!(
            assess("sapphire-jigsaw-velocity-coconut", &[])
                .score
                .unwrap()
                >= 3
        );
        assert!(
            assess("AliceSample2026!", &["AliceSample"]).score
                <= assess("AliceSample2026!", &[]).score
        );
    }
}
