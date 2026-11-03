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
use sha2::{Digest, Sha256};

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
    pub group: Pubkey,
}
impl ModelKeys {
    pub fn from_account(model: Pubkey, state: &Model) -> Self {
        Self {
            model,
            terms: state.terms,
            capacity: state.capacity,
            group: state.group,
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
    pub wallet_booking_counted: u8,
    pub provider_list: Pubkey,
    pub offer: OfferKeys,
    pub user: UserKeys,
    pub booking: Pubkey,
    pub fee_destinations: Option<[Pubkey; 2]>,
}
impl SettlementKeys {
    pub fn from_booking(
        booking: Pubkey,
        state: &Booking,
        model: ModelKeys,
        user: UserKeys,
        mint: &Pubkey,
    ) -> Result<Self> {
        ensure(
            state.model == model.model && state.user == user.user && state.owner == user.owner,
            "settlement account bindings",
        )?;
        Ok(Self {
            wallet_booking_counted: state.wallet_booking_counted,
            provider_list: state.provider_list,
            offer: OfferKeys {
                model,
                provider: state.provider,
                payout: state.payout,
                slot: state.slot,
            },
            user,
            booking,
            fee_destinations: state.fee_destinations(mint),
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
    #[serde(default)]
    pub minimum_wallet_capacity: Option<u32>,
    pub min_context: u32,
    pub limits_configured: bool,
    pub checkpoint: [u8; 32],
    pub input_modalities: u8,
    pub output_modalities: u8,
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
            self.minimum_wallet_capacity != Some(0)
                && (self.min_context == 0 || self.limits_configured)
                && self.checkpoint != [0; 32]
                && accounts::valid_modalities(self.input_modalities)
                && accounts::valid_modalities(self.output_modalities)
                && self.max_input.checked_add(self.max_output).is_some()
                && self.max_input > 0
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
        let mut p = Vec::with_capacity(146);
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
        p.extend(self.checkpoint);
        p.extend([self.input_modalities, self.output_modalities]);
        p.extend(self.min_context.to_le_bytes());
        p.push(u8::from(self.limits_configured));
        Ok(p)
    }
    pub fn select(
        &self,
        terms: &TermsBook,
        capacity: &CapacityBook,
        list: &ProviderList,
        free: u64,
    ) -> Result<Option<Selection>> {
        self.select_eligible(terms, capacity, list, free, |_| Ok(true))
    }
    pub(crate) fn select_eligible(
        &self,
        terms: &TermsBook,
        capacity: &CapacityBook,
        list: &ProviderList,
        free: u64,
        mut eligible: impl FnMut(&accounts::Offer) -> Result<bool>,
    ) -> Result<Option<Selection>> {
        self.validate()?;
        ensure(
            (10..=300).contains(&list.fee_bps) && (5000..=10_000).contains(&list.protocol_share),
            "list fees",
        )?;
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
            if t.context_window < self.min_context
                || t.checkpoint != self.checkpoint
                || t.input_modalities & self.input_modalities != self.input_modalities
                || t.output_modalities & self.output_modalities != self.output_modalities
                || self.max_input > t.max_input
                || self.max_output > t.max_output
                || self
                    .max_input
                    .checked_add(self.max_output)
                    .is_none_or(|total| total > t.context_window)
                || !t.active
                || !list.contains(&offer.provider)
                || (self.private && !t.private)
                || u32::from(t.speed_tps) < min_speed
                || cap.active >= t.capacity
                || r.input > self.rate_caps.input
                || r.cached > self.rate_caps.cached
                || r.output > self.rate_caps.output
                || !eligible(offer)?
            {
                continue;
            }
            let cached = if offer.provider == self.cache_provider {
                self.estimated_cached
            } else {
                0
            };
            let score = crate::amount::with_list_fee(
                r.charge(Usage {
                    uncached: self.estimated_input - cached,
                    cached,
                    output: self.estimated_output,
                }),
                list.fee_bps,
            )?;
            if best.as_ref().is_some_and(|best| score >= best.score) {
                continue;
            }
            let reserve = crate::amount::with_list_fee(
                r.reserve(self.max_input, self.max_output),
                list.fee_bps,
            )?;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Selection {
    pub slot: u32,
    pub reserve: u64,
    pub score: u64,
    pub cache_allowed: bool,
}

pub(crate) fn r(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
pub(crate) fn w(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
pub(crate) fn s(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
pub(crate) fn ws(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, true)
}
pub fn discriminator(name: &str) -> [u8; 8] {
    let mut hash = Sha256::new();
    hash.update(b"global:");
    hash.update(name.as_bytes());
    hash.finalize()[..8].try_into().unwrap()
}
pub(crate) fn generation_payload(generation: Option<u64>) -> Result<Vec<u8>> {
    let mut payload = vec![u8::from(generation.is_some())];
    if let Some(generation) = generation {
        ensure(generation > 0, "booking generation")?;
        payload.extend(generation.to_le_bytes());
    }
    Ok(payload)
}

impl Protocol {
    pub(crate) fn ix(
        &self,
        name: &str,
        payload: impl AsRef<[u8]>,
        accounts: Vec<AccountMeta>,
    ) -> Instruction {
        let mut data = discriminator(name).to_vec();
        data.extend(payload.as_ref());
        Instruction {
            program_id: self.program,
            accounts,
            data,
        }
    }
    pub fn initialize_market(&self, council: Pubkey) -> Instruction {
        self.ix(
            "initialize_market",
            [],
            vec![
                ws(council),
                r(self.mint),
                ws(self.market),
                r(SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn initialize_user(&self, user: UserKeys) -> Instruction {
        self.ix(
            "initialize_user",
            [],
            vec![
                ws(user.owner),
                r(self.market),
                ws(user.user),
                r(address::vault(&self.program, &user.user).0),
                r(user.vault),
                r(SYSTEM_PROGRAM),
            ],
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
        Ok(self.ix("register_offer", data, keys))
    }
    pub fn deposit(&self, u: UserKeys, source: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive deposit")?;
        Ok(self.ix(
            "deposit",
            amount.to_le_bytes(),
            vec![
                r(self.market),
                w(u.user),
                s(u.owner),
                w(source),
                r(address::vault(&self.program, &u.user).0),
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
            "update_offer",
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
            request.minimum_wallet_capacity.unwrap_or(1) == 1,
            "wallet capacity requires activated list routing",
        )?;
        ensure(
            address::model(&self.program, &self.market, &request.model_id).0 == m.model,
            "booking model id",
        )?;
        let mut p = request.encode()?;
        p.extend(generation_payload(match mode {
            BookingMode::OneShot => None,
            BookingMode::Reusable { generation, .. } => Some(generation),
        })?);
        Ok(self.ix(
            "reserve_booking",
            p,
            vec![
                r(self.market),
                r(m.model),
                r(m.terms),
                w(m.capacity),
                w(u.user),
                s(u.owner),
                w(booking),
                r(rent_payer),
                r(request.provider_list),
                r(if m.group == Pubkey::default() {
                    self.program
                } else {
                    m.group
                }),
            ],
        ))
    }
    pub fn claim(
        &self,
        k: SettlementKeys,
        usage: Usage,
        receipt: Option<&Receipt>,
    ) -> Result<Instruction> {
        ensure(
            matches!(k.wallet_booking_counted, 0 | 2 | 3),
            "wallet booking marker",
        )?;
        let o = k.offer;
        let u = k.user;
        let mut p = Vec::with_capacity(140);
        usage.encode(&mut p);
        let mut keys = vec![
            r(self.market),
            r(o.model.model),
            w(self.settlement_capacity(k)),
            w(u.user),
            w(k.booking),
            s(o.provider),
            w(u.vault),
            w(o.payout),
            r(self.mint),
            r(address::vault(&self.program, &u.user).0),
            r(TOKEN_PROGRAM),
        ];
        if k.wallet_booking_counted > 0 {
            keys.push(self.wallet_booking_meta(k));
        }
        p.push(u8::from(receipt.is_some()));
        if let Some(c) = receipt {
            ensure(
                c.owner == u.owner && c.user == u.user && c.booking == k.booking,
                "claim receipt binding",
            )?;
            ensure(c.generation > 0, "claim generation")?;
            p.extend(c.encode());
        }
        if let Some(destinations) = k.fee_destinations {
            keys.extend(destinations.map(w));
        }
        Ok(self.ix(
            if k.wallet_booking_counted == 0 {
                "claim_booking"
            } else {
                "claim_list_booking"
            },
            p,
            keys,
        ))
    }
    pub fn expire(&self, k: SettlementKeys, generation: Option<u64>) -> Result<Instruction> {
        ensure(
            matches!(k.wallet_booking_counted, 0 | 2 | 3),
            "wallet booking marker",
        )?;
        let mut keys = self.settlement_keys(k);
        if k.wallet_booking_counted > 0 {
            keys.push(self.wallet_booking_meta(k));
        }
        Ok(self.ix(
            if k.wallet_booking_counted == 0 {
                "expire_booking"
            } else {
                "expire_list_booking"
            },
            generation_payload(generation)?,
            keys,
        ))
    }
    pub fn withdraw(&self, u: UserKeys, destination: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive withdrawal")?;
        Ok(self.ix(
            "withdraw",
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
        ensure(
            matches!(k.wallet_booking_counted, 0 | 2 | 3),
            "wallet booking marker",
        )?;
        let mut keys = self.settlement_keys(k);
        keys.push(s(k.offer.provider));
        if k.wallet_booking_counted > 0 {
            keys.push(self.wallet_booking_meta(k));
        }
        Ok(self.ix(
            if k.wallet_booking_counted == 0 {
                "release_booking"
            } else {
                "release_list_booking"
            },
            generation_payload(generation)?,
            keys,
        ))
    }
    pub fn close(&self, account: Pubkey, rent_payer: Pubkey, reusable: bool) -> Instruction {
        self.ix(
            "close_booking",
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
            s(k.provider),
            ws(k.payer),
            r(address::provider(&self.program, &self.market, &k.provider).0),
            w(k.model.model),
            ws(k.model.terms),
            ws(k.model.capacity),
            w(address::model_metadata(&self.program, &k.model.model).0),
            r(SYSTEM_PROGRAM),
        ];
        Ok(self.ix("initialize_model", p, keys))
    }
    pub fn initialize_provider(
        &self,
        provider: Pubkey,
        payer: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            "initialize_provider",
            document.encode()?,
            vec![
                r(self.market),
                s(provider),
                ws(payer),
                w(address::provider(&self.program, &self.market, &provider).0),
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
            "update_profile",
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
            "update_model_metadata",
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
            "initialize_receipt",
            [],
            vec![
                r(self.market),
                r(u.user),
                s(u.owner),
                ws(payer),
                ws(booking),
                r(SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn initialize_list(
        &self,
        list: Pubkey,
        authority: Pubkey,
        config: ListCreation,
    ) -> Result<Instruction> {
        list_size(config.limit)?;
        ensure(
            (10..=300).contains(&config.fee_bps)
                && (5000..=10_000).contains(&config.protocol_share),
            "list fees",
        )?;
        ensure(config.maximum_payment > 0, "list creation maximum payment")?;
        let mut payload = config.limit.to_le_bytes().to_vec();
        payload.extend(config.fee_bps.to_le_bytes());
        payload.extend(config.protocol_share.to_le_bytes());
        payload.extend(config.maximum_payment.to_le_bytes());
        Ok(self.ix(
            "initialize_list",
            payload,
            vec![
                r(self.market),
                ws(authority),
                ws(list),
                r(SYSTEM_PROGRAM),
                w(config.source),
                w(config.destination),
                r(config.mint),
                r(config.token_program),
            ],
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
        self.ix(
            "update_list",
            payload,
            vec![r(self.market), w(list), s(authority)],
        )
    }
    pub fn close_list(&self, list: Pubkey, authority: Pubkey) -> Instruction {
        self.ix(
            "close_list",
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
            "initialize_list_metadata",
            document.encode()?,
            vec![
                r(self.market),
                r(list),
                s(authority),
                ws(payer),
                w(address::list_metadata(&self.program, &list).0),
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
            "update_list_metadata",
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
            "grow_books",
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
    fn wallet_booking_meta(&self, k: SettlementKeys) -> AccountMeta {
        match k.wallet_booking_counted {
            2 | 3 => w(address::provider_wallet(&self.program, &self.market, &k.user.owner).0),
            _ => r(self.program),
        }
    }
    fn settlement_capacity(&self, k: SettlementKeys) -> Pubkey {
        if k.wallet_booking_counted == 3 {
            address::provider_limit_page(&self.program, &self.market, (k.offer.slot & 31) as u8).0
        } else if k.wallet_booking_counted == 2 {
            address::list_capacity(&self.program, &k.provider_list, &k.offer.model.model).0
        } else {
            k.offer.model.capacity
        }
    }
    fn settlement_keys(&self, k: SettlementKeys) -> Vec<AccountMeta> {
        vec![
            r(self.market),
            r(k.offer.model.model),
            w(self.settlement_capacity(k)),
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
        accounts::HEADER_LEN + limit as usize * 132,
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

#[derive(Clone, Copy, Debug)]
pub struct ListCreation {
    pub limit: u32,
    pub fee_bps: u16,
    pub protocol_share: u16,
    pub maximum_payment: u64,
    pub source: Pubkey,
    pub destination: Pubkey,
    pub mint: Pubkey,
    pub token_program: Pubkey,
}
