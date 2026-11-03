use crate::{
    Instruction, Pubkey, Result,
    accounts::{
        Capacity, CapacityBook, Decode, ProviderList, RawAccount, TermsBook, i64le, key, n32, state,
    },
    address,
    booking_limits::slab,
    ensure,
    instruction::{
        BookRequest, BookingMode, ModelKeys, Protocol, Selection, UserKeys, r, s, w, ws,
    },
    scoped::{ListRouting, ListTerms, ProviderWallet},
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

pub const PAGE_COUNT: u8 = 32;
pub const PAGE_HEADER_BYTES: usize = 48;
pub const ENTRY_BYTES: usize = 40;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct RequestLimits {
    pub concurrency: u32,
    pub quota: u32,
    pub quota_window_seconds: u32,
}
impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            concurrency: 0,
            quota: 0,
            quota_window_seconds: 60,
        }
    }
}
impl RequestLimits {
    pub fn validate(self) -> Result<()> {
        ensure(
            (1..=86_400).contains(&self.quota_window_seconds),
            "quota window seconds",
        )
    }
}
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct ProviderLimits {
    pub global: RequestLimits,
    pub wallet: RequestLimits,
}
impl ProviderLimits {
    pub fn encode(self) -> Result<Vec<u8>> {
        self.global.validate()?;
        self.wallet.validate()?;
        borsh::to_vec(&self).map_err(|_| crate::Error::Invalid("provider limits encoding"))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderLimitEntry {
    pub concurrency_limit: u32,
    pub quota_limit: u32,
    pub quota_window_seconds: u32,
    pub quota_window_end: i64,
    pub quota_used: u32,
    pub active_bookings: u32,
    pub wallet_concurrency_limit: u32,
    pub wallet_quota_limit: u32,
    pub wallet_quota_window_seconds: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderLimitPage {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub page: u8,
    pub capacity: u32,
    pub entries: Vec<ProviderLimitEntry>,
}
impl ProviderLimitEntry {
    pub fn limits(&self) -> ProviderLimits {
        ProviderLimits {
            global: RequestLimits {
                concurrency: if self.concurrency_limit == u32::MAX {
                    0
                } else {
                    self.concurrency_limit
                },
                quota: self.quota_limit,
                quota_window_seconds: self.quota_window_seconds,
            },
            wallet: RequestLimits {
                concurrency: self.wallet_concurrency_limit,
                quota: self.wallet_quota_limit,
                quota_window_seconds: self.wallet_quota_window_seconds,
            },
        }
    }
}
impl Decode for ProviderLimitPage {
    const MAGIC: &'static [u8; 8] = &[18, 101, 138, 157, 3, 31, 233, 252];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, capacity, count) = slab(a, p, Self::MAGIC, PAGE_HEADER_BYTES, ENTRY_BYTES)?;
        ensure(
            d[41] < PAGE_COUNT
                && d[42..44] == [0; 2]
                && address::provider_limit_page(p, &key(d, 8), d[41]) == (*at, d[40]),
            "provider limit page identity",
        )?;
        let entries = d[48..48 + count * ENTRY_BYTES]
            .as_chunks::<ENTRY_BYTES>()
            .0
            .iter()
            .map(|row| {
                let entry = ProviderLimitEntry {
                    concurrency_limit: n32(row, 0),
                    quota_limit: n32(row, 4),
                    quota_window_seconds: n32(row, 8),
                    quota_window_end: i64le(row, 12),
                    quota_used: n32(row, 20),
                    active_bookings: n32(row, 24),
                    wallet_concurrency_limit: n32(row, 28),
                    wallet_quota_limit: n32(row, 32),
                    wallet_quota_window_seconds: n32(row, 36),
                };
                ensure(
                    entry.concurrency_limit > 0
                        && (1..=86_400).contains(&entry.quota_window_seconds)
                        && (1..=86_400).contains(&entry.wallet_quota_window_seconds),
                    "provider limit entry",
                )?;
                Ok(entry)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            market: key(d, 8),
            page: d[41],
            capacity,
            entries,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderLimitIndex {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub index: u32,
}
impl Decode for ProviderLimitIndex {
    const MAGIC: &'static [u8; 8] = &[94, 216, 132, 219, 87, 112, 166, 89];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, 80)?;
        ensure(
            key(d, 40) != Pubkey::default()
                && d[77..] == [0; 3]
                && address::provider_limits(p, &key(d, 8), &key(d, 40)) == (*at, d[76]),
            "provider limit index identity",
        )?;
        Ok(Self {
            market: key(d, 8),
            provider: key(d, 40),
            index: n32(d, 72),
        })
    }
}
pub fn required_pages(terms: &ListTerms) -> Vec<u8> {
    let mut pages = terms
        .entries
        .iter()
        .map(|e| (e.offer.provider_policy & 31) as u8)
        .collect::<Vec<_>>();
    pages.sort_unstable();
    pages.dedup();
    pages
}
impl Protocol {
    pub fn activate_provider_limits(&self, council: Pubkey) -> Instruction {
        self.ix(
            "activate_provider_limits",
            [],
            vec![w(self.market), s(council)],
        )
    }
    pub fn initialize_provider_limit_page(
        &self,
        payer: Pubkey,
        page: u8,
        limit: u32,
    ) -> Result<Instruction> {
        ensure(
            page < PAGE_COUNT && limit > 0 && 48 + u64::from(limit) * 40 <= 10_240,
            "initial provider limit page",
        )?;
        let mut data = limit.to_le_bytes().to_vec();
        data.push(page);
        Ok(self.ix(
            "initialize_provider_limit_page",
            data,
            vec![
                r(self.market),
                ws(payer),
                w(address::provider_limit_page(&self.program, &self.market, page).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn grow_provider_limit_page(
        &self,
        payer: Pubkey,
        page: u8,
        limit: u32,
    ) -> Result<Instruction> {
        ensure(
            page < PAGE_COUNT && limit > 0 && 48 + u64::from(limit) * 40 <= 10 * 1024 * 1024,
            "provider limit page growth",
        )?;
        Ok(self.ix(
            "grow_provider_limit_page",
            limit.to_le_bytes(),
            vec![
                w(address::provider_limit_page(&self.program, &self.market, page).0),
                ws(payer),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn initialize_provider_limits(
        &self,
        provider: Pubkey,
        payer: Pubkey,
        page: u8,
        limits: ProviderLimits,
    ) -> Result<Instruction> {
        ensure(page < PAGE_COUNT, "provider limit page")?;
        Ok(self.ix(
            "initialize_provider_limits",
            limits.encode()?,
            vec![
                s(provider),
                ws(payer),
                r(address::provider(&self.program, &self.market, &provider).0),
                w(address::provider_limit_page(&self.program, &self.market, page).0),
                w(address::provider_limits(&self.program, &self.market, &provider).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn configure_provider_limits(
        &self,
        provider: Pubkey,
        page: u8,
        limits: ProviderLimits,
    ) -> Result<Instruction> {
        ensure(page < PAGE_COUNT, "provider limit page")?;
        Ok(self.ix(
            "configure_provider_limits",
            limits.encode()?,
            vec![
                s(provider),
                r(address::provider_limits(&self.program, &self.market, &provider).0),
                r(address::provider(&self.program, &self.market, &provider).0),
                w(address::provider_limit_page(&self.program, &self.market, page).0),
            ],
        ))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn book_with_provider_limits(
        &self,
        m: ModelKeys,
        u: UserKeys,
        booking: Pubkey,
        payer: Pubkey,
        request: &BookRequest,
        mode: BookingMode,
        pages: &[u8],
    ) -> Result<Instruction> {
        let mut pages = pages.to_vec();
        pages.sort_unstable();
        pages.dedup();
        ensure(
            !pages.is_empty() && pages.iter().all(|p| *p < PAGE_COUNT),
            "booking provider limit pages",
        )?;
        let mut ix = self.book_scoped(m, u, booking, payer, request, mode)?;
        ix.accounts[3] = w(address::provider_limit_page(&self.program, &self.market, pages[0]).0);
        ix.accounts.extend(
            pages[1..]
                .iter()
                .map(|page| w(address::provider_limit_page(&self.program, &self.market, *page).0)),
        );
        Ok(ix)
    }
    pub fn set_provider_list_offer(
        &self,
        list: Pubkey,
        model: Pubkey,
        provider: Pubkey,
        payout: Pubkey,
        terms: crate::accounts::OfferTerms,
    ) -> Result<Instruction> {
        let mut ix = self.set_list_offer(list, model, provider, payout, terms)?;
        ix.accounts[3] = r(address::provider_limits(&self.program, &self.market, &provider).0);
        Ok(ix)
    }
    pub fn prune_provider_list_offer(
        &self,
        list: Pubkey,
        model: Pubkey,
        provider: Pubkey,
    ) -> Instruction {
        let mut ix = self.prune_list_offer(list, model, provider);
        ix.accounts[3] = r(provider);
        ix
    }
}
impl BookRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn select_provider_limits(
        &self,
        terms: &ListTerms,
        pages: &[ProviderLimitPage],
        list: &ProviderList,
        routing: &ListRouting,
        wallet: Option<&ProviderWallet>,
        free: u64,
        now: i64,
    ) -> Result<Option<Selection>> {
        ensure(
            terms.limits_version == 1
                && terms.list == self.provider_list
                && routing.list == terms.list
                && list.market == routing.market
                && wallet.is_none_or(|w| w.market == routing.market),
            "provider limit selection bindings",
        )?;
        let mut indexed = [None; PAGE_COUNT as usize];
        for page in pages {
            ensure(
                page.market == routing.market
                    && page.page < PAGE_COUNT
                    && page.entries.len() <= page.capacity as usize,
                "provider limit page binding",
            )?;
            ensure(
                indexed[page.page as usize].replace(page).is_none(),
                "duplicate provider limit page",
            )?;
        }
        let mut ids = std::collections::HashSet::new();
        ensure(
            terms
                .entries
                .iter()
                .all(|entry| ids.insert(entry.offer.provider_policy)),
            "duplicate provider limit id",
        )?;
        let mut order = (0..terms.entries.len()).collect::<Vec<_>>();
        order.sort_unstable_by_key(|&i| terms.entries[i].order);
        let mut entries = Vec::with_capacity(order.len());
        let mut active = Vec::with_capacity(order.len());
        for &i in &order {
            let mut offer = terms.entries[i].offer.clone();
            let id = offer.provider_policy;
            let row = indexed[(id & 31) as usize]
                .and_then(|p| p.entries.get((id >> 5) as usize))
                .ok_or(crate::Error::Invalid("missing provider limit page or row"))?;
            offer.terms.capacity = row.concurrency_limit;
            active.push(Capacity {
                active: row.active_bookings,
            });
            entries.push(offer);
        }
        let plain = TermsBook {
            model: terms.model,
            limit: terms.limit,
            entries,
        };
        let capacity = CapacityBook {
            model: terms.model,
            limit: terms.limit,
            entries: active,
        };
        let selected = self.select_eligible(&plain, &capacity, list, free, |offer| {
            let id = offer.provider_policy;
            let row = indexed[(id & 31) as usize]
                .and_then(|p| p.entries.get((id >> 5) as usize))
                .unwrap();
            let minimum = self.minimum_wallet_capacity.unwrap_or(1);
            if !routing
                .entries
                .binary_search_by_key(&offer.provider, |e| e.provider)
                .ok()
                .is_some_and(|i| routing.entries[i].active)
                || row.concurrency_limit < minimum
                || row.quota_limit != 0
                    && now < row.quota_window_end
                    && row.quota_used >= row.quota_limit
                || row.wallet_concurrency_limit != 0 && row.wallet_concurrency_limit < minimum
            {
                return Ok(false);
            }
            let usage = wallet.and_then(|w| {
                w.entries
                    .binary_search_by_key(&offer.provider, |e| e.provider)
                    .ok()
                    .map(|i| &w.entries[i])
            });
            Ok(usage.is_none_or(|u| {
                (row.wallet_concurrency_limit == 0 || u.outstanding < row.wallet_concurrency_limit)
                    && (row.wallet_quota_limit == 0
                        || now >= u.window_end
                        || u.bookings < row.wallet_quota_limit)
            }))
        })?;
        Ok(selected.map(|mut v| {
            v.slot = order[v.slot as usize] as u32;
            v
        }))
    }
}
