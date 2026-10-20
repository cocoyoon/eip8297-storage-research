//! SplitMix64: deterministic, dependency-free RNG for reproducible runs.

pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn digest(&mut self) -> [u8; 32] {
        let mut v = [0u8; 32];
        for i in 0..4 {
            v[i * 8..(i + 1) * 8].copy_from_slice(&self.next().to_be_bytes());
        }
        v
    }
    /// Random nonzero 32-byte value.
    pub fn value(&mut self) -> [u8; 32] {
        let mut v = self.digest();
        v[31] |= 1;
        v
    }
    pub fn address(&mut self) -> [u8; 32] {
        let mut a = [0u8; 32];
        // 20-byte-style address padded to 32, as address20_to_address32 would.
        for i in 0..20 {
            a[12 + i] = (self.next() & 0xFF) as u8;
        }
        a
    }
}

/// Zipf sampler over ranks 0..n with exponent s, via cumulative weights.
pub struct Zipf {
    cum: Vec<f64>,
}

impl Zipf {
    pub fn new(n: usize, s: f64) -> Self {
        let mut cum = Vec::with_capacity(n);
        let mut acc = 0.0;
        for i in 0..n {
            acc += 1.0 / ((i + 1) as f64).powf(s);
            cum.push(acc);
        }
        Zipf { cum }
    }
    pub fn len(&self) -> usize {
        self.cum.len()
    }
    /// Extend to cover newly added ranks (weights continue the series).
    pub fn grow_to(&mut self, n: usize, s: f64) {
        let mut acc = *self.cum.last().unwrap_or(&0.0);
        for i in self.cum.len()..n {
            acc += 1.0 / ((i + 1) as f64).powf(s);
            self.cum.push(acc);
        }
    }
    pub fn sample(&self, rng: &mut Rng) -> usize {
        let total = *self.cum.last().expect("empty zipf");
        let r = rng.f64() * total;
        match self.cum.binary_search_by(|c| c.partial_cmp(&r).unwrap()) {
            Ok(i) => i,
            Err(i) => i.min(self.cum.len() - 1),
        }
    }
}
