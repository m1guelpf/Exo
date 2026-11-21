//! A small deterministic PRNG (SplitMix64). Fixtures must be byte-identical for a seed across
//! platforms and dependency upgrades, which rules out `rand`'s unspecified generators.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    /// Uniform in `low..=high`.
    pub fn between(&mut self, low: i64, high: i64) -> i64 {
        let span = u64::try_from(high - low).unwrap_or(0) + 1;
        low + i64::try_from(self.below(span)).unwrap_or(0)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let index = usize::try_from(self.below(items.len() as u64)).unwrap_or(0);
        &items[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!(
            (0..4).map(|_| a.next_u64()).collect::<Vec<_>>(),
            (0..4).map(|_| b.next_u64()).collect::<Vec<_>>()
        );
        assert_ne!(Rng::new(7).next_u64(), Rng::new(8).next_u64());
    }
}
