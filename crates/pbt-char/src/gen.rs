//! Synthetic workload generator.
//!
//! IMPORTANT: every distribution here is an ASSUMPTION standing in for
//! mainnet replay, which Phase 1 specifies but this session cannot source.
//! The generator exists to validate the characterization pipeline and to
//! produce a first, clearly-labeled synthetic zone x depth distribution.
//! All parameters are CLI-overridable; the report records them.

use crate::rng::{Rng, Zipf};
use pbt_ref::hash::{h, Digest, KECCAK_EMPTY, ZERO_VALUE};
use pbt_ref::keys::*;

pub struct Params {
    pub seed: u64,
    pub eoas: usize,
    pub contracts: usize,
    pub templates: usize,
    pub blocks: usize,
    pub ops_per_block: usize,
    /// op mix, out of 1000: transfer / call / churn / deploy / selfdestruct
    pub mix_transfer: u64,
    pub mix_call: u64,
    pub mix_churn: u64,
    pub mix_deploy: u64,
    pub mix_selfdestruct: u64,
    pub zipf_s: f64,
    /// Contracts with mainnet-style mega storage (tens of thousands of slots),
    /// created first so they take the hottest Zipf ranks.
    pub mega_contracts: usize,
    /// Probability (out of 100) that a storage update writes the value the
    /// slot already holds -- mainnet same-value SSTOREs are a double-digit
    /// percentage of storage writes; these must be engine no-ops.
    pub same_value_pct: u64,
    /// Zipf exponent for WHICH EOA a transfer or call touches. 0 keeps the
    /// original behaviour: uniform over all EOAs, which is the least
    /// cache-friendly choice there is. Bounded-cache results depend on this
    /// more than on any other knob, so it is swept rather than fixed.
    pub eoa_zipf_s: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            seed: 8297,
            eoas: 50_000,
            contracts: 6_000,
            templates: 300,
            blocks: 1_000,
            ops_per_block: 200,
            mix_transfer: 580,
            mix_call: 320,
            mix_churn: 60,
            mix_deploy: 30,
            mix_selfdestruct: 10,
            zipf_s: 1.0,
            mega_contracts: 3,
            same_value_pct: 15,
            eoa_zipf_s: 0.0,
        }
    }
}

struct Eoa {
    address: Address32,
    nonce: u64,
    balance: u128,
}

struct Contract {
    address: Address32,
    template: usize,
    /// live hot slots (header, sub-index < 64) with current values
    hot: Vec<(u8, Digest)>,
    /// live overflow slots (>= 64) with current values
    overflow: Vec<(StorageSlot, Digest)>,
    nonce: u64,
    balance: u128,
}

struct Template {
    code_hash: Digest,
    /// (key, value) of every nonzero chunk
    chunks: Vec<(Vec<u8>, Digest)>,
    holders: u64,
}

/// One state write: key + value (zero value = deletion).
pub type Write = (Vec<u8>, Digest);

pub struct World {
    pub params: Params,
    rng: Rng,
    eoas: Vec<Eoa>,
    contracts: Vec<Contract>,
    templates: Vec<Template>,
    contract_zipf: Zipf,
    template_zipf: Zipf,
    /// None = uniform, drawn with the same RNG calls as before the knob
    /// existed, so default runs reproduce bit for bit.
    eoa_zipf: Option<Zipf>,
}

impl World {
    pub fn new(params: Params) -> Self {
        let rng = Rng::new(params.seed);
        let contract_zipf = Zipf::new(params.contracts, params.zipf_s);
        let template_zipf = Zipf::new(params.templates, params.zipf_s);
        let eoa_zipf = (params.eoa_zipf_s > 0.0).then(|| Zipf::new(params.eoas, params.eoa_zipf_s));
        World {
            params,
            rng,
            eoas: Vec::new(),
            contracts: Vec::new(),
            templates: Vec::new(),
            contract_zipf,
            template_zipf,
            eoa_zipf,
        }
    }

    fn pick_eoa(&mut self) -> usize {
        let n = self.eoas.len();
        match &self.eoa_zipf {
            Some(z) => z.sample(&mut self.rng).min(n - 1),
            None => self.rng.below(n as u64) as usize,
        }
    }

    fn synth_code(rng: &mut Rng) -> Vec<u8> {
        // Size: 500..8500 bytes, skewed small. ~20% PUSH opcodes with data.
        let size = 500 + (rng.below(90) * rng.below(90)) as usize;
        let mut code = Vec::with_capacity(size);
        while code.len() < size {
            if rng.below(5) == 0 {
                let n = 1 + rng.below(32) as u8;
                code.push(PUSH_OFFSET + n);
                for _ in 0..n {
                    code.push((rng.next() & 0xFF) as u8);
                }
            } else {
                code.push((1 + rng.below(0x50)) as u8); // non-PUSH opcode range
            }
        }
        code.truncate(size);
        code
    }

    /// Build the initial state; returns the full list of initial writes.
    pub fn genesis(&mut self) -> Vec<Write> {
        let mut writes: Vec<Write> = Vec::new();

        // Templates (shared code).
        for _ in 0..self.params.templates {
            let code = Self::synth_code(&mut self.rng);
            let code_hash = h(&code); // pseudo code-hash (BLAKE3, labeled)
            let mut chunks = Vec::new();
            for (i, chunk) in chunkify_code(&code).into_iter().enumerate() {
                if chunk != ZERO_VALUE {
                    chunks.push((get_tree_key_for_code_chunk(&code_hash, i as u64), chunk));
                }
            }
            self.templates.push(Template {
                code_hash,
                chunks,
                holders: 0,
            });
        }

        // EOAs.
        for _ in 0..self.params.eoas {
            let address = self.rng.address();
            let nonce = self.rng.below(100);
            let balance = self.rng.next() as u128;
            writes.push((
                get_tree_key_for_basic_data(&address),
                pack_basic_data(0, nonce, balance),
            ));
            writes.push((get_tree_key_for_code_hash(&address), KECCAK_EMPTY));
            self.eoas.push(Eoa {
                address,
                nonce,
                balance,
            });
        }

        // Contracts. Mega contracts first, so they occupy the hottest Zipf
        // ranks (mainnet's hottest contracts are also its largest).
        for _ in 0..self.params.mega_contracts {
            let template = self.rng.below(self.params.templates as u64) as usize;
            let n_over = 30_000 + self.rng.below(50_000) as usize;
            let w = self.deploy_contract_sized(template, Some(n_over));
            writes.extend(w);
        }
        for _ in 0..self
            .params
            .contracts
            .saturating_sub(self.params.mega_contracts)
        {
            let template = self.rng.below(self.params.templates as u64) as usize;
            let w = self.deploy_contract(template);
            writes.extend(w);
        }

        writes
    }

    /// Deploy a contract of the given template; returns its writes.
    fn deploy_contract(&mut self, template: usize) -> Vec<Write> {
        self.deploy_contract_sized(template, None)
    }

    fn deploy_contract_sized(&mut self, template: usize, overflow_n: Option<usize>) -> Vec<Write> {
        let mut writes = Vec::new();
        let address = self.rng.address();
        let nonce = 1u64;
        let balance = self.rng.below(1 << 30) as u128;

        let tpl = &mut self.templates[template];
        let code_size = 0u32; // size folded into code_hash for this model
        writes.push((
            get_tree_key_for_basic_data(&address),
            pack_basic_data(code_size, nonce, balance),
        ));
        writes.push((get_tree_key_for_code_hash(&address), tpl.code_hash));
        // Content-addressed chunks: emitted every deploy; dedup shows up as
        // no-op writes in the engine when the leaves already exist.
        writes.extend(tpl.chunks.iter().cloned());
        tpl.holders += 1;

        // Hot header slots, skewed low count.
        let n_hot = 1 + (self.rng.below(8) * self.rng.below(8) / 8).min(63) as usize;
        let mut hot: Vec<(u8, Digest)> = Vec::new();
        for _ in 0..n_hot {
            let s = self.rng.below(HEADER_STORAGE_SLOTS as u64) as u8;
            if !hot.iter().any(|(h, _)| *h == s) {
                let v = self.rng.value();
                writes.push((
                    get_tree_key_for_storage_slot(&address, &slot_from_u64(s as u64)),
                    v,
                ));
                hot.push((s, v));
            }
        }

        // Overflow slots: 60% of contracts, skewed count (or fixed for mega).
        let mut overflow: Vec<(StorageSlot, Digest)> = Vec::new();
        let n_over = match overflow_n {
            Some(n) => n,
            None if self.rng.below(10) < 6 => (self.rng.below(25) * self.rng.below(16)) as usize,
            None => 0,
        };
        for _ in 0..n_over {
            let slot = Self::mapping_slot(&mut self.rng);
            let v = self.rng.value();
            writes.push((get_tree_key_for_storage_slot(&address, &slot), v));
            overflow.push((slot, v));
        }

        self.contracts.push(Contract {
            address,
            template,
            hot,
            overflow,
            nonce,
            balance,
        });
        writes
    }

    /// Mapping-style slot: keccak-derived in reality; any 32-byte number
    /// with high entropy lands in the storage zone. tree_index huge.
    fn mapping_slot(rng: &mut Rng) -> StorageSlot {
        rng.digest()
    }

    /// Generate one block of writes.
    pub fn block(&mut self) -> Vec<Write> {
        let mut writes = Vec::new();
        let mix_total = self.params.mix_transfer
            + self.params.mix_call
            + self.params.mix_churn
            + self.params.mix_deploy
            + self.params.mix_selfdestruct;
        for _ in 0..self.params.ops_per_block {
            let r = self.rng.below(mix_total);
            if r < self.params.mix_transfer {
                self.op_transfer(&mut writes);
            } else if r < self.params.mix_transfer + self.params.mix_call {
                self.op_call(&mut writes);
            } else if r < self.params.mix_transfer + self.params.mix_call + self.params.mix_churn {
                self.op_churn(&mut writes);
            } else if r < mix_total - self.params.mix_selfdestruct {
                self.op_deploy(&mut writes);
            } else {
                self.op_selfdestruct_ephemeral(&mut writes);
            }
        }
        writes
    }

    fn op_transfer(&mut self, writes: &mut Vec<Write>) {
        let si = self.pick_eoa();
        let amount = (self.rng.below(1 << 20)) as u128;
        {
            let s = &mut self.eoas[si];
            s.nonce += 1;
            s.balance = s.balance.saturating_sub(amount);
            writes.push((
                get_tree_key_for_basic_data(&s.address),
                pack_basic_data(0, s.nonce, s.balance),
            ));
        }
        // Recipient: EOA 80% / contract 20%.
        if self.rng.below(10) < 8 || self.contracts.is_empty() {
            let ri = self.pick_eoa();
            let r = &mut self.eoas[ri];
            r.balance = r.balance.saturating_add(amount);
            writes.push((
                get_tree_key_for_basic_data(&r.address),
                pack_basic_data(0, r.nonce, r.balance),
            ));
        } else {
            let ri = self
                .contract_zipf
                .sample(&mut self.rng)
                .min(self.contracts.len() - 1);
            let c = &mut self.contracts[ri];
            c.balance = c.balance.saturating_add(amount);
            writes.push((
                get_tree_key_for_basic_data(&c.address),
                pack_basic_data(0, c.nonce, c.balance),
            ));
        }
    }

    fn op_call(&mut self, writes: &mut Vec<Write>) {
        // Caller EOA pays gas: basic_data write.
        let si = self.pick_eoa();
        {
            let s = &mut self.eoas[si];
            s.nonce += 1;
            writes.push((
                get_tree_key_for_basic_data(&s.address),
                pack_basic_data(0, s.nonce, s.balance),
            ));
        }
        let ci = self
            .contract_zipf
            .sample(&mut self.rng)
            .min(self.contracts.len() - 1);
        let same_pct = self.params.same_value_pct;
        // Hot header slots: 1..=2 writes; same-value with probability same_pct.
        let n_hot = 1 + self.rng.below(2) as usize;
        for _ in 0..n_hot {
            if self.contracts[ci].hot.is_empty() {
                break;
            }
            let hi = self.rng.below(self.contracts[ci].hot.len() as u64) as usize;
            let same = self.rng.below(100) < same_pct;
            let addr = self.contracts[ci].address;
            let (s, cur) = self.contracts[ci].hot[hi];
            let v = if same { cur } else { self.rng.value() };
            writes.push((
                get_tree_key_for_storage_slot(&addr, &slot_from_u64(s as u64)),
                v,
            ));
            self.contracts[ci].hot[hi].1 = v;
        }
        // Overflow: 0..=2 ops: 70% update / 20% create / 10% zero-out.
        let n_over = self.rng.below(3) as usize;
        for _ in 0..n_over {
            let r = self.rng.below(10);
            let addr = self.contracts[ci].address;
            if r < 7 {
                if self.contracts[ci].overflow.is_empty() {
                    continue;
                }
                let oi = self.rng.below(self.contracts[ci].overflow.len() as u64) as usize;
                let same = self.rng.below(100) < same_pct;
                let (slot, cur) = self.contracts[ci].overflow[oi];
                let v = if same { cur } else { self.rng.value() };
                writes.push((get_tree_key_for_storage_slot(&addr, &slot), v));
                self.contracts[ci].overflow[oi].1 = v;
            } else if r < 9 {
                let slot = Self::mapping_slot(&mut self.rng);
                let v = self.rng.value();
                writes.push((get_tree_key_for_storage_slot(&addr, &slot), v));
                self.contracts[ci].overflow.push((slot, v));
            } else {
                let c = &mut self.contracts[ci];
                if c.overflow.is_empty() {
                    continue;
                }
                let i = self.rng.below(c.overflow.len() as u64) as usize;
                let (slot, _) = c.overflow.swap_remove(i);
                writes.push((get_tree_key_for_storage_slot(&addr, &slot), ZERO_VALUE));
            }
        }
    }

    fn op_churn(&mut self, writes: &mut Vec<Write>) {
        // Cold access: uniform over contracts, one overflow op.
        let ci = self.rng.below(self.contracts.len() as u64) as usize;
        let addr = self.contracts[ci].address;
        if self.rng.below(2) == 0 || self.contracts[ci].overflow.is_empty() {
            let slot = Self::mapping_slot(&mut self.rng);
            let v = self.rng.value();
            writes.push((get_tree_key_for_storage_slot(&addr, &slot), v));
            self.contracts[ci].overflow.push((slot, v));
        } else {
            let oi = self.rng.below(self.contracts[ci].overflow.len() as u64) as usize;
            let v = self.rng.value();
            let (slot, _) = self.contracts[ci].overflow[oi];
            writes.push((get_tree_key_for_storage_slot(&addr, &slot), v));
            self.contracts[ci].overflow[oi].1 = v;
        }
    }

    fn op_deploy(&mut self, writes: &mut Vec<Write>) {
        // 85% reuse an existing template (dedup), 15% fresh code.
        let template = if self.rng.below(100) < 85 {
            self.template_zipf
                .sample(&mut self.rng)
                .min(self.templates.len() - 1)
        } else {
            let code = Self::synth_code(&mut self.rng);
            let code_hash = h(&code);
            let mut chunks = Vec::new();
            for (i, chunk) in chunkify_code(&code).into_iter().enumerate() {
                if chunk != ZERO_VALUE {
                    chunks.push((get_tree_key_for_code_chunk(&code_hash, i as u64), chunk));
                }
            }
            self.templates.push(Template {
                code_hash,
                chunks,
                holders: 0,
            });
            self.template_zipf
                .grow_to(self.templates.len(), self.params.zipf_s);
            self.templates.len() - 1
        };
        let w = self.deploy_contract(template);
        writes.extend(w);
        self.contract_zipf
            .grow_to(self.contracts.len(), self.params.zipf_s);
    }

    /// EIP-6780-style: contract created and self-destructed within the same
    /// block. Header and storage leaves are removed; code chunks are removed
    /// only if no other account holds the template (spec's transaction-local
    /// rule -- the world model plays the STF's role here).
    fn op_selfdestruct_ephemeral(&mut self, writes: &mut Vec<Write>) {
        let template = self
            .template_zipf
            .sample(&mut self.rng)
            .min(self.templates.len() - 1);
        let w = self.deploy_contract(template);
        writes.extend(w);
        let c = self.contracts.pop().expect("just deployed");
        self.contract_zipf
            .grow_to(self.contracts.len() + 1, self.params.zipf_s);
        // Remove header leaves.
        writes.push((get_tree_key_for_basic_data(&c.address), ZERO_VALUE));
        writes.push((get_tree_key_for_code_hash(&c.address), ZERO_VALUE));
        for (s, _) in &c.hot {
            writes.push((
                get_tree_key_for_storage_slot(&c.address, &slot_from_u64(*s as u64)),
                ZERO_VALUE,
            ));
        }
        for (slot, _) in &c.overflow {
            writes.push((get_tree_key_for_storage_slot(&c.address, slot), ZERO_VALUE));
        }
        // Code: keep unless this deploy was the only holder.
        let tpl = &mut self.templates[template];
        tpl.holders -= 1;
        if tpl.holders == 0 {
            for (k, _) in tpl.chunks.clone() {
                writes.push((k, ZERO_VALUE));
            }
        }
    }
}
