use crate::{
    AccountMeta, Instruction, Pubkey, Result,
    accounts::{
        self, Booking, CapacityBook, DocumentRef, Model, OfferTerms, ProviderList, TermsBook, User,
    },
    address::{self, ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM, TOKEN_PROGRAM},
    amount::{Rates, Usage},
    ensure,
    receipt::Receipt,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    #[serde(with = "crate::serde_pubkey")]
    pub program: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelKeys {
    pub model: Pubkey,
    pub terms: Pubkey,
    pub capacity: Pubkey,
}
impl ModelKeys {
    pub fn from_account(model: Pubkey, state: &Model) -> Self {
        Self {
            model,
            terms: state.terms,
            capacity: state.capacity,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserKeys {
    pub user: Pubkey,
    pub owner: Pubkey,
    pub vault: Pubkey,
}
impl UserKeys {
    pub fn from_account(user: Pubkey, state: &User) -> Self {
        Self {
            user,
            owner: state.owner,
            vault: state.vault,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfferKeys {
    pub model: ModelKeys,
    pub provider: Pubkey,
    pub payout: Pubkey,
    pub slot: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettlementKeys {
    pub offer: OfferKeys,
    pub user: UserKeys,
    pub booking: Pubkey,
}
impl SettlementKeys {
    pub fn from_booking(
        booking: Pubkey,
        state: &Booking,
        model: ModelKeys,
        user: UserKeys,
    ) -> Result<Self> {
        ensure(
            state.model == model.model && state.user == user.user && state.owner == user.owner,
            "settlement account bindings",
        )?;
        Ok(Self {
            offer: OfferKeys {
                model,
                provider: state.provider,
                payout: state.payout,
                slot: state.slot,
            },
            user,
            booking,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookingMode {
    OneShot,
    Reusable { generation: u64, fresh: bool },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRequest {
    pub model_id: [u8; 32],
    #[serde(with = "crate::serde_pubkey")]
    pub provider_list: Pubkey,
    pub rate_caps: Rates,
    pub min_speed: u16,
    pub max_input: u32,
    pub max_output: u32,
    pub private: bool,
    pub max_spend: u64,
    pub estimated_input: u32,
    pub estimated_cached: u32,
    pub estimated_output: u32,
    #[serde(with = "crate::serde_pubkey")]
    pub cache_provider: Pubkey,
}
impl BookRequest {
    pub fn validate(&self) -> Result<()> {
        ensure(
            self.max_input > 0
                && self.max_output > 0
                && self.estimated_input <= self.max_input
                && self.estimated_output <= self.max_output
                && self.estimated_cached <= self.estimated_input
                && (self.estimated_cached == 0 || self.cache_provider != Pubkey::default()),
            "booking request",
        )
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut p = Vec::with_capacity(115);
        p.extend(self.model_id);
        self.rate_caps.encode(&mut p);
        p.extend(self.min_speed.to_le_bytes());
        p.extend(self.max_input.to_le_bytes());
        p.extend(self.max_output.to_le_bytes());
        p.push(u8::from(self.private));
        p.extend(self.max_spend.to_le_bytes());
        for n in [
            self.estimated_input,
            self.estimated_cached,
            self.estimated_output,
        ] {
            p.extend(n.to_le_bytes());
        }
        p.extend(self.cache_provider.as_ref());
        Ok(p)
    }
    pub fn select(
        &self,
        terms: &TermsBook,
        capacity: &CapacityBook,
        list: &ProviderList,
        free: u64,
    ) -> Result<Option<Selection>> {
        self.validate()?;
        ensure(
            terms.model == capacity.model
                && terms.limit == capacity.limit
                && terms.entries.len() == capacity.entries.len(),
            "offer book pair",
        )?;
        let min_speed = u32::from(self.min_speed).max(self.max_output.div_ceil(86_400 - 300));
        let mut best: Option<Selection> = None;
        for (slot, (offer, cap)) in terms.entries.iter().zip(&capacity.entries).enumerate() {
            let t = offer.terms;
            let r = t.rates;
            if !t.active
                || !list.contains(&offer.provider)
                || (self.private && !t.private)
                || u32::from(t.speed_tps) < min_speed
                || cap.active >= t.capacity
                || r.input > self.rate_caps.input
                || r.cached > self.rate_caps.cached
                || r.output > self.rate_caps.output
            {
                continue;
            }
            let cached = if offer.provider == self.cache_provider {
                self.estimated_cached
            } else {
                0
            };
            let score = r.charge(Usage {
                uncached: self.estimated_input - cached,
                cached,
                output: self.estimated_output,
            });
            if best.as_ref().is_some_and(|best| score >= best.score) {
                continue;
            }
            let reserve = r.reserve(self.max_input, self.max_output);
            if reserve > free.min(self.max_spend) {
                continue;
            }
            best = Some(Selection {
                slot: slot as u32,
                reserve,
                score,
                cache_allowed: offer.provider == self.cache_provider,
            });
            if score == 0 {
                break;
            }
        }
        Ok(best)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub slot: u32,
    pub reserve: u64,
    pub score: u64,
    pub cache_allowed: bool,
}

fn r(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn w(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn s(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
fn ws(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, true)
}
fn generation_payload(generation: Option<u64>) -> Result<Vec<u8>> {
    if let Some(n) = generation {
        ensure(n > 0 && n < (1u64 << 56), "booking generation")?;
        Ok(n.to_le_bytes().to_vec())
    } else {
        Ok(vec![])
    }
}

impl Protocol {
    fn ix(&self, op: u8, p: impl AsRef<[u8]>, accounts: Vec<AccountMeta>) -> Instruction {
        let p = p.as_ref();
        let mut data = Vec::with_capacity(p.len() + 1);
        data.push(op);
        data.extend(p);
        Instruction {
            program_id: self.program,
            accounts,
            data,
        }
    }
    pub fn initialize_market(&self, admin: Pubkey) -> Instruction {
        self.ix(0, [], vec![ws(self.market), s(admin), r(self.mint)])
    }
    pub fn initialize_user(&self, user: UserKeys) -> Instruction {
        self.ix(
            1,
            [],
            vec![ws(user.user), s(user.owner), r(self.market), r(user.vault)],
        )
    }
    pub fn register_offer(&self, k: OfferKeys, terms: OfferTerms) -> Result<Instruction> {
        let mut data = k.slot.to_le_bytes().to_vec();
        data.extend(terms.encode()?);
        let keys = vec![
            r(self.market),
            r(k.model.model),
            w(k.model.terms),
            w(k.model.capacity),
            r(address::provider(&self.program, &self.market, &k.provider).0),
            s(k.provider),
            r(k.payout),
        ];
        Ok(self.ix(2, data, keys))
    }
    pub fn deposit(&self, u: UserKeys, source: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive deposit")?;
        Ok(self.ix(
            3,
            amount.to_le_bytes(),
            vec![
                r(self.market),
                w(u.user),
                s(u.owner),
                w(source),
                w(u.vault),
                r(self.mint),
                r(TOKEN_PROGRAM),
            ],
        ))
    }
    pub fn update_offer(&self, k: OfferKeys, terms: OfferTerms) -> Result<Instruction> {
        let mut p = k.slot.to_le_bytes().to_vec();
        p.extend(terms.encode()?);
        Ok(self.ix(
            5,
            p,
            vec![
                r(self.market),
                r(k.model.model),
                w(k.model.terms),
                s(k.provider),
            ],
        ))
    }
    pub fn book(
        &self,
        m: ModelKeys,
        u: UserKeys,
        booking: Pubkey,
        rent_payer: Pubkey,
        request: &BookRequest,
        mode: BookingMode,
    ) -> Result<Instruction> {
        ensure(
            address::model(&self.program, &self.market, &request.model_id).0 == m.model,
            "booking model id",
        )?;
        let mut p = request.encode()?;
        let (booking_meta, payer_meta) = match mode {
            BookingMode::OneShot => (ws(booking), s(rent_payer)),
            BookingMode::Reusable { generation, fresh } => {
                p.extend(generation_payload(Some(generation))?);
                (AccountMeta::new(booking, fresh), r(rent_payer))
            }
        };
        Ok(self.ix(
            6,
            p,
            vec![
                r(self.market),
                r(m.model),
                r(m.terms),
                w(m.capacity),
                w(u.user),
                s(u.owner),
                booking_meta,
                payer_meta,
                r(request.provider_list),
            ],
        ))
    }
    pub fn claim(
        &self,
        k: SettlementKeys,
        usage: Usage,
        receipt: Option<&Receipt>,
    ) -> Result<Instruction> {
        let o = k.offer;
        let u = k.user;
        let mut p = Vec::with_capacity(140);
        usage.encode(&mut p);
        let keys = vec![
            r(self.market),
            r(o.model.model),
            w(o.model.capacity),
            w(u.user),
            w(k.booking),
            s(o.provider),
            w(u.vault),
            w(o.payout),
            r(self.mint),
            r(address::vault(&self.program, &u.user).0),
            r(TOKEN_PROGRAM),
        ];
        if let Some(c) = receipt {
            ensure(
                c.owner == u.owner && c.user == u.user && c.booking == k.booking,
                "claim receipt binding",
            )?;
            p.extend(generation_payload(Some(c.generation))?);
            p.extend(c.encode());
        }
        Ok(self.ix(7, p, keys))
    }
    pub fn expire(&self, k: SettlementKeys, generation: Option<u64>) -> Result<Instruction> {
        Ok(self.ix(8, generation_payload(generation)?, self.settlement_keys(k)))
    }
    pub fn withdraw(&self, u: UserKeys, destination: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive withdrawal")?;
        Ok(self.ix(
            10,
            amount.to_le_bytes(),
            vec![
                r(self.market),
                w(u.user),
                s(u.owner),
                w(u.vault),
                w(destination),
                r(self.mint),
                r(address::vault(&self.program, &u.user).0),
                r(TOKEN_PROGRAM),
            ],
        ))
    }
    pub fn release(&self, k: SettlementKeys, generation: Option<u64>) -> Result<Instruction> {
        let mut keys = self.settlement_keys(k);
        keys.push(s(k.offer.provider));
        Ok(self.ix(15, generation_payload(generation)?, keys))
    }
    pub fn close(&self, account: Pubkey, rent_payer: Pubkey, reusable: bool) -> Instruction {
        self.ix(
            17,
            [],
            vec![w(account), AccountMeta::new(rent_payer, reusable)],
        )
    }
    pub fn initialize_model(&self, k: InitializeModel<'_>) -> Result<Instruction> {
        book_sizes(k.limit)?;
        ensure(
            k.model.terms != k.model.capacity
                && address::model(&self.program, &self.market, &k.id).0 == k.model.model,
            "new model bindings",
        )?;
        let mut p = k.id.to_vec();
        p.extend(k.limit.to_le_bytes());
        p.extend(k.document.encode()?);
        let keys = vec![
            r(self.market),
            w(k.model.model),
            ws(k.model.terms),
            ws(k.model.capacity),
            s(k.provider),
            ws(k.payer),
            r(SYSTEM_PROGRAM),
            r(address::provider(&self.program, &self.market, &k.provider).0),
            w(address::model_metadata(&self.program, &k.model.model).0),
        ];
        Ok(self.ix(18, p, keys))
    }
    pub fn initialize_provider(
        &self,
        provider: Pubkey,
        payer: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            19,
            document.encode()?,
            vec![
                r(self.market),
                w(address::provider(&self.program, &self.market, &provider).0),
                s(provider),
                ws(payer),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn update_provider_metadata(
        &self,
        provider: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            21,
            document.encode()?,
            vec![
                r(self.market),
                w(address::provider(&self.program, &self.market, &provider).0),
                s(provider),
            ],
        ))
    }
    pub fn update_model_metadata(
        &self,
        model: Pubkey,
        creator: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            22,
            document.encode()?,
            vec![
                r(self.market),
                r(model),
                w(address::model_metadata(&self.program, &model).0),
                s(creator),
            ],
        ))
    }
    pub fn initialize_reusable_booking(
        &self,
        u: UserKeys,
        booking: Pubkey,
        payer: Pubkey,
    ) -> Instruction {
        self.ix(
            26,
            [],
            vec![r(self.market), r(u.user), s(u.owner), ws(booking), s(payer)],
        )
    }
    pub fn initialize_list(
        &self,
        list: Pubkey,
        authority: Pubkey,
        limit: u32,
    ) -> Result<Instruction> {
        list_size(limit)?;
        Ok(self.ix(
            28,
            limit.to_le_bytes(),
            vec![r(self.market), ws(list), s(authority)],
        ))
    }
    pub fn update_list(
        &self,
        list: Pubkey,
        authority: Pubkey,
        provider: Pubkey,
        included: bool,
    ) -> Instruction {
        let mut payload = provider.to_bytes().to_vec();
        payload.push(u8::from(included));
        self.ix(29, payload, vec![r(self.market), w(list), s(authority)])
    }
    pub fn close_list(&self, list: Pubkey, authority: Pubkey) -> Instruction {
        self.ix(
            30,
            [],
            vec![
                w(list),
                ws(authority),
                w(address::list_metadata(&self.program, &list).0),
            ],
        )
    }
    pub fn initialize_list_metadata(
        &self,
        list: Pubkey,
        authority: Pubkey,
        payer: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            33,
            document.encode()?,
            vec![
                r(self.market),
                r(list),
                w(address::list_metadata(&self.program, &list).0),
                s(authority),
                ws(payer),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn update_list_metadata(
        &self,
        list: Pubkey,
        authority: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            34,
            document.encode()?,
            vec![
                r(self.market),
                r(list),
                w(address::list_metadata(&self.program, &list).0),
                s(authority),
            ],
        ))
    }
    pub fn grow_model(&self, model: ModelKeys, payer: Pubkey, limit: u32) -> Result<Instruction> {
        ensure(
            (2..=accounts::MAX_ENTRIES).contains(&limit),
            "new model book limit",
        )?;
        Ok(self.ix(
            32,
            limit.to_le_bytes(),
            vec![
                r(self.market),
                w(model.model),
                w(model.terms),
                w(model.capacity),
                ws(payer),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    fn settlement_keys(&self, k: SettlementKeys) -> Vec<AccountMeta> {
        vec![
            r(self.market),
            r(k.offer.model.model),
            w(k.offer.model.capacity),
            w(k.user.user),
            w(k.booking),
        ]
    }
    pub fn create_account(
        &self,
        payer: Pubkey,
        account: Pubkey,
        lamports: u64,
        space: usize,
    ) -> Result<Instruction> {
        create_account(payer, account, self.program, lamports, space)
    }
}

pub struct InitializeModel<'a> {
    pub model: ModelKeys,
    pub provider: Pubkey,
    pub payer: Pubkey,
    pub id: [u8; 32],
    pub limit: u32,
    pub document: &'a DocumentRef,
}
pub fn book_sizes(limit: u32) -> Result<[usize; 2]> {
    ensure(
        (1..=accounts::MAX_ENTRIES).contains(&limit),
        "model book limit",
    )?;
    Ok([
        accounts::HEADER_LEN + limit as usize * accounts::TERMS_LEN,
        accounts::HEADER_LEN + limit as usize * accounts::CAPACITY_LEN,
    ])
}
pub fn create_account(
    payer: Pubkey,
    account: Pubkey,
    owner: Pubkey,
    lamports: u64,
    space: usize,
) -> Result<Instruction> {
    let size = u64::try_from(space).map_err(|_| crate::Error::Arithmetic)?;
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend(lamports.to_le_bytes());
    data.extend(size.to_le_bytes());
    data.extend(owner.as_ref());
    Ok(Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![ws(payer), ws(account)],
        data,
    })
}
pub fn create_associated_token(payer: Pubkey, owner: Pubkey, mint: Pubkey) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM,
        accounts: vec![
            ws(payer),
            w(address::associated_token(&mint, &owner)),
            r(owner),
            r(mint),
            r(SYSTEM_PROGRAM),
            r(TOKEN_PROGRAM),
        ],
        data: vec![1],
    }
}

pub fn list_size(limit: u32) -> Result<usize> {
    ensure(
        (1..=accounts::MAX_ENTRIES).contains(&limit),
        "provider list capacity",
    )?;
    Ok(accounts::LIST_HEADER_LEN + limit as usize * accounts::LIST_SLOT_LEN)
}
