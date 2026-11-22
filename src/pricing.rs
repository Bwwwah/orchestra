//! What a session would cost at API prices, in dollars: an estimate from the tokens read in its
//! conversation file. The output price per million tokens comes from the catalog (`classify`,
//! in cents); the input price is a fifth of it, a cache write costs 1.25 times the input and a
//! cache read a tenth. Used next to the subscription percentages for AIs run with an API key.

/// Amounts are kept in micro-dollars (millionths) so that sums stay exact.
pub const MICRO: f64 = 1_000_000.0;

/// The tokens of one exchange, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

/// Micro-dollars for `c` on the model named `model` (an unknown name counts as intermediate).
pub fn cost_micro(model: &str, c: Counts) -> u64 {
    let (_, out_cents) = crate::catalog::classify(model, "");
    // Dollars per million tokens.
    let out = out_cents as f64 / 100.0;
    let input = out / 5.0;
    let usd = (c.input as f64 * input
        + c.output as f64 * out
        + c.cache_write as f64 * input * 1.25
        + c.cache_read as f64 * input * 0.1)
        / 1e6;
    (usd * MICRO).round() as u64
}

/// "$0,42", "$12,3", "< $0,01"; nothing for zero.
pub fn dollars(micro: u64) -> String {
    let usd = micro as f64 / MICRO;
    if micro == 0 {
        String::new()
    } else if usd < 0.01 {
        "< $0,01".to_string()
    } else if usd < 10.0 {
        format!("${usd:.2}").replace('.', ",")
    } else {
        format!("${usd:.1}").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_million_output_tokens_cost_the_output_price() {
        // Opus: 2000 cents per million output tokens.
        let c = Counts {
            output: 1_000_000,
            ..Counts::default()
        };
        assert_eq!(cost_micro("claude-opus-5-5", c), 20_000_000);
    }

    #[test]
    fn cache_reads_are_a_tenth_of_input_and_writes_a_quarter_above() {
        let m = "claude-sonnet-5-5"; // output $10, input $2
        let read = Counts {
            cache_read: 1_000_000,
            ..Counts::default()
        };
        let write = Counts {
            cache_write: 1_000_000,
            ..Counts::default()
        };
        assert_eq!(cost_micro(m, read), 200_000);
        assert_eq!(cost_micro(m, write), 2_500_000);
    }

    #[test]
    fn amounts_are_written_short() {
        assert_eq!(dollars(0), "");
        assert_eq!(dollars(3_000), "< $0,01");
        assert_eq!(dollars(420_000), "$0,42");
        assert_eq!(dollars(12_340_000), "$12,3");
    }
}
