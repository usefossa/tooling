use crate::{
    Instruction, Pubkey, Result,
    accounts::{
        BookingLimits, Capacity, CapacityBook, Decode, Offer, OfferTerms, ProviderList, RawAccount,
        TermsBook, i64le, key, n16, n32, n64, state,
    },
    address,
    booking_limits::slab,
    ensure,
    instruction::{
        BookRequest, BookingMode, ModelKeys, Protocol, Selection, UserKeys, generation_payload, r,
        s, w, ws,
    },
};
use serde::Serialize;

fn limits(d: &[u8], at: usize) -> Result<BookingLimits> {
    let value = BookingLimits {
        wallet_window_seconds: n32(d, at),
        wallet_bookings_per_window: n32(d, at + 4),
        wallet_booking_capacity: n32(d, at + 8),
    };
    value.encode()?;
    Ok(value)
}
fn sorted(keys: impl Iterator<Item = Pubkey>) -> Result<()> {
    let mut last = None;
    for key in keys {
        ensure(
            key != Pubkey::default() && last.is_none_or(|last| last < key),
            "sorted routing providers",
        )?;
        last = Some(key);
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderPolicy {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub limits: BookingLimits,
    pub epoch: u64,
    pub active_mirrors: u32,
    pub updating: bool,
}
impl Decode for ProviderPolicy {
    const MAGIC: &'static [u8; 8] = &[160, 80, 153, 123, 160, 152, 122, 32];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, 100)?;
        ensure(
            address::provider_policy(p, &key(d, 8), &key(d, 40)) == (*at, d[97])
                && d[98..] == [0; 2]
                && d[96] <= 1
                && n64(d, 84) > 0,
            "provider policy identity",
        )?;
        Ok(Self {
            market: key(d, 8),
            provider: key(d, 40),
            limits: limits(d, 72)?,
            epoch: n64(d, 84),
            active_mirrors: n32(d, 92),
            updating: d[96] == 1,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderModelCapacity {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub total: u32,
    pub allocated: u32,
    pub legacy_active: u32,
    pub legacy_slot: Option<u32>,
}
impl Decode for ProviderModelCapacity {
    const MAGIC: &'static [u8; 8] = &[139, 242, 225, 128, 103, 72, 36, 45];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, 92)?;
        ensure(
            address::provider_capacity(p, &key(d, 8), &key(d, 40)) == (*at, d[88])
                && d[89..] == [0; 3]
                && u64::from(n32(d, 76)) + u64::from(n32(d, 80)) <= u64::from(n32(d, 72)),
            "provider capacity identity/budget",
        )?;
        Ok(Self {
            model: key(d, 8),
            provider: key(d, 40),
            total: n32(d, 72),
            allocated: n32(d, 76),
            legacy_active: n32(d, 80),
            legacy_slot: (n32(d, 84) != u32::MAX).then_some(n32(d, 84)),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PolicyMirror {
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub limits: BookingLimits,
    pub epoch: u64,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListRouting {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub list: Pubkey,
    pub capacity: u32,
    pub book_count: u32,
    pub entries: Vec<PolicyMirror>,
}
impl Decode for ListRouting {
    const MAGIC: &'static [u8; 8] = &[144, 176, 101, 27, 253, 191, 157, 242];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, capacity, count) = slab(a, p, Self::MAGIC, 84, 56)?;
        ensure(
            capacity <= 4096
                && address::list_routing(p, &key(d, 40)) == (*at, d[76])
                && d[77..80] == [0; 3],
            "list routing identity",
        )?;
        let entries = d[84..84 + count * 56]
            .as_chunks::<56>()
            .0
            .iter()
            .map(|row| {
                ensure(row[52] <= 1 && row[53..] == [0; 3], "policy mirror flags")?;
                Ok(PolicyMirror {
                    provider: key(row, 0),
                    limits: limits(row, 32)?,
                    epoch: n64(row, 44),
                    active: row[52] == 1,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        sorted(entries.iter().map(|e| e.provider))?;
        Ok(Self {
            market: key(d, 8),
            list: key(d, 40),
            capacity,
            book_count: n32(d, 72),
            entries,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListOffer {
    pub offer: Offer,
    pub order: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListTerms {
    pub limits_version: u8,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub list: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub capacity_address: Pubkey,
    pub limit: u32,
    pub next_order: u32,
    pub entries: Vec<ListOffer>,
}
impl Decode for ListTerms {
    const MAGIC: &'static [u8; 8] = &[174, 240, 243, 180, 203, 181, 0, 107];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, capacity, count) = slab(a, p, Self::MAGIC, 120, 140)?;
        ensure(
            capacity <= 4096
                && capacity == n32(d, 104)
                && address::list_terms(p, &key(d, 40), &key(d, 8)) == (*at, d[112])
                && address::list_capacity(p, &key(d, 40), &key(d, 8)).0 == key(d, 72)
                && d[113] <= 1
                && d[114..116] == [0; 2],
            "list terms identity",
        )?;
        let entries = d[120..120 + count * 140]
            .as_chunks::<140>()
            .0
            .iter()
            .map(|row| {
                ensure(
                    matches!(row[114], 1 | 2)
                        && (d[113] == 0 || row[114] == 2)
                        && row[115] == 0
                        && row[110] <= 1
                        && row[111] <= 1
                        && n32(row, 136) < n32(d, 108),
                    "list offer layout/order",
                )?;
                let terms = OfferTerms {
                    checkpoint: row[64..96].try_into().unwrap(),
                    input_modalities: row[112],
                    output_modalities: row[113],
                    context_window: n32(row, 120),
                    max_input: n32(row, 124),
                    max_output: n32(row, 128),
                    rates: crate::amount::Rates {
                        input: n32(row, 96),
                        cached: n32(row, 100),
                        output: n32(row, 104),
                    },
                    speed_tps: n16(row, 108),
                    private: row[110] == 1,
                    active: row[111] == 1,
                    capacity: n32(row, 116),
                };
                OfferTerms {
                    capacity: terms.capacity.max(1),
                    ..terms
                }
                .validate()?;
                Ok(ListOffer {
                    offer: Offer {
                        provider_policy: n32(row, 132),
                        provider: key(row, 0),
                        payout: key(row, 32),
                        terms,
                    },
                    order: n32(row, 136),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        sorted(entries.iter().map(|e| e.offer.provider))?;
        ensure(
            entries
                .iter()
                .map(|e| e.order)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == entries.len(),
            "unique offer order",
        )?;
        Ok(Self {
            limits_version: d[113],
            model: key(d, 8),
            list: key(d, 40),
            capacity_address: key(d, 72),
            limit: capacity,
            next_order: n32(d, 108),
            entries,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListCapacityEntry {
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub active: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListCapacity {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub list: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub terms_address: Pubkey,
    pub limit: u32,
    pub entries: Vec<ListCapacityEntry>,
}
impl Decode for ListCapacity {
    const MAGIC: &'static [u8; 8] = &[73, 4, 213, 31, 149, 94, 219, 141];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, capacity, count) = slab(a, p, Self::MAGIC, 116, 36)?;
        ensure(
            capacity <= 4096
                && capacity == n32(d, 104)
                && address::list_capacity(p, &key(d, 40), &key(d, 8)) == (*at, d[108])
                && address::list_terms(p, &key(d, 40), &key(d, 8)).0 == key(d, 72)
                && d[109..112] == [0; 3],
            "list capacity identity",
        )?;
        let entries = d[116..116 + count * 36]
            .as_chunks::<36>()
            .0
            .iter()
            .map(|row| ListCapacityEntry {
                provider: key(row, 0),
                active: n32(row, 32),
            })
            .collect::<Vec<_>>();
        sorted(entries.iter().map(|e| e.provider))?;
        Ok(Self {
            model: key(d, 8),
            list: key(d, 40),
            terms_address: key(d, 72),
            limit: capacity,
            entries,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderUsage {
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub window_end: i64,
    pub bookings: u32,
    pub outstanding: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderWallet {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub rent_payer: Pubkey,
    pub capacity: u32,
    pub entries: Vec<ProviderUsage>,
}
impl Decode for ProviderWallet {
    const MAGIC: &'static [u8; 8] = &[89, 4, 132, 220, 207, 78, 178, 52];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, capacity, count) = slab(a, p, Self::MAGIC, 112, 48)?;
        ensure(
            address::provider_wallet(p, &key(d, 8), &key(d, 40)) == (*at, d[104])
                && d[105..108] == [0; 3],
            "provider wallet identity",
        )?;
        let entries = d[112..112 + count * 48]
            .as_chunks::<48>()
            .0
            .iter()
            .map(|row| ProviderUsage {
                provider: key(row, 0),
                window_end: i64le(row, 32),
                bookings: n32(row, 40),
                outstanding: n32(row, 44),
            })
            .collect::<Vec<_>>();
        sorted(entries.iter().map(|e| e.provider))?;
        Ok(Self {
            market: key(d, 8),
            owner: key(d, 40),
            rent_payer: key(d, 72),
            capacity,
            entries,
        })
    }
}
impl BookRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn select_scoped(
        &self,
        terms: &ListTerms,
        capacity: &ListCapacity,
        list: &ProviderList,
        routing: &ListRouting,
        wallet: Option<&ProviderWallet>,
        free: u64,
        now: i64,
    ) -> Result<Option<Selection>> {
        ensure(
            terms.model == capacity.model
                && self.provider_list == terms.list
                && terms.list == capacity.list
                && terms.list == routing.list
                && list.market == routing.market
                && terms.limit == capacity.limit
                && terms.entries.len() == capacity.entries.len()
                && terms
                    .entries
                    .iter()
                    .zip(&capacity.entries)
                    .all(|(a, b)| a.offer.provider == b.provider)
                && wallet.is_none_or(|w| w.market == routing.market),
            "scoped booking bindings",
        )?;
        let mut order = (0..terms.entries.len()).collect::<Vec<_>>();
        order.sort_unstable_by_key(|&index| terms.entries[index].order);
        let plain = TermsBook {
            model: terms.model,
            limit: terms.limit,
            entries: order
                .iter()
                .map(|&i| terms.entries[i].offer.clone())
                .collect(),
        };
        let active = CapacityBook {
            model: capacity.model,
            limit: capacity.limit,
            entries: order
                .iter()
                .map(|&i| Capacity {
                    active: capacity.entries[i].active,
                })
                .collect(),
        };
        let selected = self.select_eligible(&plain, &active, list, free, |offer| {
            let Some(policy) = routing
                .entries
                .binary_search_by_key(&offer.provider, |entry| entry.provider)
                .ok()
                .map(|i| &routing.entries[i])
                .filter(|policy| {
                    policy.active
                        && (policy.limits.wallet_booking_capacity == 0
                            || policy.limits.wallet_booking_capacity
                                >= self.minimum_wallet_capacity.unwrap_or(1))
                        && offer.terms.capacity >= self.minimum_wallet_capacity.unwrap_or(1)
                })
            else {
                return Ok(false);
            };
            let usage = wallet.and_then(|w| {
                w.entries
                    .binary_search_by_key(&offer.provider, |entry| entry.provider)
                    .ok()
                    .map(|i| &w.entries[i])
            });
            Ok(usage.is_none_or(|u| {
                (policy.limits.wallet_booking_capacity == 0
                    || u.outstanding < policy.limits.wallet_booking_capacity)
                    && (policy.limits.wallet_bookings_per_window == 0
                        || now >= u.window_end
                        || u.bookings < policy.limits.wallet_bookings_per_window)
            }))
        })?;
        Ok(selected.map(|mut value| {
            value.slot = order[value.slot as usize] as u32;
            value
        }))
    }
}

impl Protocol {
    pub fn activate_list_routing(&self, council: Pubkey) -> Instruction {
        self.ix(
            "activate_list_routing",
            [],
            vec![w(self.market), s(council)],
        )
    }
    pub fn initialize_provider_policy(
        &self,
        provider: Pubkey,
        payer: Pubkey,
        limits: BookingLimits,
    ) -> Result<Instruction> {
        Ok(self.ix(
            "initialize_provider_policy",
            limits.encode()?,
            vec![
                r(self.market),
                s(provider),
                ws(payer),
                r(address::provider(&self.program, &self.market, &provider).0),
                w(address::provider_policy(&self.program, &self.market, &provider).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn begin_provider_policy_update(&self, provider: Pubkey) -> Instruction {
        self.ix(
            "begin_provider_policy_update",
            [],
            vec![
                w(address::provider_policy(&self.program, &self.market, &provider).0),
                s(provider),
            ],
        )
    }
    pub fn commit_provider_policy_update(
        &self,
        provider: Pubkey,
        limits: BookingLimits,
    ) -> Result<Instruction> {
        Ok(self.ix(
            "commit_provider_policy_update",
            limits.encode()?,
            vec![
                w(address::provider_policy(&self.program, &self.market, &provider).0),
                s(provider),
            ],
        ))
    }
    pub fn initialize_list_routing(
        &self,
        list: Pubkey,
        authority: Pubkey,
        payer: Pubkey,
        limit: u32,
    ) -> Result<Instruction> {
        ensure(
            limit > 0 && 84 + u64::from(limit) * 56 <= 10_240,
            "initial list routing capacity",
        )?;
        Ok(self.ix(
            "initialize_list_routing",
            limit.to_le_bytes(),
            vec![
                r(self.market),
                w(list),
                s(authority),
                ws(payer),
                w(address::list_routing(&self.program, &list).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn grow_list_routing(
        &self,
        list: Pubkey,
        authority: Pubkey,
        payer: Pubkey,
        limit: u32,
    ) -> Instruction {
        self.ix(
            "grow_list_routing",
            limit.to_le_bytes(),
            vec![
                r(list),
                s(authority),
                w(address::list_routing(&self.program, &list).0),
                ws(payer),
                r(address::SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn update_routing_list(
        &self,
        list: Pubkey,
        authority: Pubkey,
        provider: Pubkey,
        enabled: bool,
        policy: bool,
    ) -> Instruction {
        let mut data = provider.to_bytes().to_vec();
        data.push(u8::from(enabled));
        self.ix(
            "update_routing_list",
            data,
            vec![
                w(list),
                s(authority),
                w(address::list_routing(&self.program, &list).0),
                if policy {
                    w(address::provider_policy(&self.program, &self.market, &provider).0)
                } else {
                    r(self.program)
                },
            ],
        )
    }
    pub fn enable_policy_mirror(&self, list: Pubkey, provider: Pubkey) -> Instruction {
        self.ix(
            "enable_policy_mirror",
            [],
            vec![
                s(provider),
                w(address::provider_policy(&self.program, &self.market, &provider).0),
                r(list),
                w(address::list_routing(&self.program, &list).0),
            ],
        )
    }
    pub fn freeze_policy_mirror(
        &self,
        list: Pubkey,
        provider: Pubkey,
        signer: Pubkey,
    ) -> Instruction {
        self.ix(
            "freeze_policy_mirror",
            [],
            vec![
                s(signer),
                w(address::provider_policy(&self.program, &self.market, &provider).0),
                r(list),
                w(address::list_routing(&self.program, &list).0),
            ],
        )
    }
    pub fn close_list_routing(&self, list: Pubkey, authority: Pubkey) -> Instruction {
        self.ix(
            "close_list_routing",
            [],
            vec![
                w(list),
                ws(authority),
                w(address::list_routing(&self.program, &list).0),
            ],
        )
    }
    pub fn initialize_model_capacity(
        &self,
        model: ModelKeys,
        provider: Pubkey,
        payer: Pubkey,
        total: u32,
    ) -> Instruction {
        self.ix(
            "initialize_model_capacity",
            total.to_le_bytes(),
            vec![
                r(self.market),
                r(model.model),
                s(provider),
                ws(payer),
                r(address::provider(&self.program, &self.market, &provider).0),
                r(model.terms),
                r(model.capacity),
                w(address::provider_capacity(&self.program, &model.model, &provider).0),
                r(address::SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn configure_model_capacity(
        &self,
        model: ModelKeys,
        provider: Pubkey,
        total: u32,
    ) -> Instruction {
        self.ix(
            "configure_model_capacity",
            total.to_le_bytes(),
            vec![
                s(provider),
                w(address::provider_capacity(&self.program, &model.model, &provider).0),
                r(model.model),
                r(model.capacity),
            ],
        )
    }
    pub fn sync_legacy_capacity(&self, model: ModelKeys, provider: Pubkey) -> Instruction {
        self.ix(
            "sync_legacy_capacity",
            [],
            vec![
                w(address::provider_capacity(&self.program, &model.model, &provider).0),
                r(model.model),
                r(model.capacity),
            ],
        )
    }
    pub fn initialize_list_books(
        &self,
        list: Pubkey,
        model: Pubkey,
        authority: Pubkey,
        payer: Pubkey,
        limit: u32,
    ) -> Result<Instruction> {
        ensure(
            limit > 0 && 120 + u64::from(limit) * 140 <= 10_240,
            "initial list book capacity",
        )?;
        Ok(self.ix(
            "initialize_list_books",
            limit.to_le_bytes(),
            vec![
                r(self.market),
                r(model),
                r(list),
                s(authority),
                w(address::list_routing(&self.program, &list).0),
                ws(payer),
                w(address::list_terms(&self.program, &list, &model).0),
                w(address::list_capacity(&self.program, &list, &model).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn grow_list_books(
        &self,
        list: Pubkey,
        model: Pubkey,
        authority: Pubkey,
        payer: Pubkey,
        limit: u32,
    ) -> Instruction {
        self.ix(
            "grow_list_books",
            limit.to_le_bytes(),
            vec![
                r(list),
                s(authority),
                w(address::list_terms(&self.program, &list, &model).0),
                w(address::list_capacity(&self.program, &list, &model).0),
                ws(payer),
                r(address::SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn close_list_books(&self, list: Pubkey, model: Pubkey, authority: Pubkey) -> Instruction {
        self.ix(
            "close_list_books",
            [],
            vec![
                r(list),
                ws(authority),
                w(address::list_routing(&self.program, &list).0),
                w(address::list_terms(&self.program, &list, &model).0),
                w(address::list_capacity(&self.program, &list, &model).0),
            ],
        )
    }
    pub fn set_list_offer(
        &self,
        list: Pubkey,
        model: Pubkey,
        provider: Pubkey,
        payout: Pubkey,
        terms: OfferTerms,
    ) -> Result<Instruction> {
        let capacity = terms.capacity;
        let mut encoded = OfferTerms {
            capacity: capacity.max(1),
            ..terms
        }
        .encode()?;
        encoded[16..20].copy_from_slice(&capacity.to_le_bytes());
        let mut data = 0u32.to_le_bytes().to_vec();
        data.extend(encoded);
        Ok(self.ix(
            "set_list_offer",
            data,
            vec![
                r(self.market),
                r(model),
                s(provider),
                w(address::provider_capacity(&self.program, &model, &provider).0),
                r(list),
                r(address::list_routing(&self.program, &list).0),
                w(address::list_terms(&self.program, &list, &model).0),
                w(address::list_capacity(&self.program, &list, &model).0),
                r(payout),
            ],
        ))
    }
    pub fn prune_list_offer(&self, list: Pubkey, model: Pubkey, provider: Pubkey) -> Instruction {
        self.ix(
            "prune_list_offer",
            [],
            vec![
                r(list),
                w(address::list_terms(&self.program, &list, &model).0),
                w(address::list_capacity(&self.program, &list, &model).0),
                w(address::provider_capacity(&self.program, &model, &provider).0),
            ],
        )
    }
    pub fn initialize_provider_wallet(
        &self,
        owner: Pubkey,
        payer: Pubkey,
        limit: u32,
    ) -> Result<Instruction> {
        ensure(
            limit > 0 && 112 + u64::from(limit) * 48 <= 10_240,
            "initial provider wallet capacity",
        )?;
        Ok(self.ix(
            "initialize_provider_wallet",
            limit.to_le_bytes(),
            vec![
                r(self.market),
                s(owner),
                ws(payer),
                w(address::provider_wallet(&self.program, &self.market, &owner).0),
                r(address::SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn grow_provider_wallet(&self, owner: Pubkey, payer: Pubkey, limit: u32) -> Instruction {
        self.ix(
            "grow_provider_wallet",
            limit.to_le_bytes(),
            vec![
                s(owner),
                w(address::provider_wallet(&self.program, &self.market, &owner).0),
                ws(payer),
                r(address::SYSTEM_PROGRAM),
            ],
        )
    }
    pub fn prune_provider_wallet(&self, owner: Pubkey) -> Instruction {
        self.ix(
            "prune_provider_wallet",
            [],
            vec![
                s(owner),
                w(address::provider_wallet(&self.program, &self.market, &owner).0),
            ],
        )
    }
    pub fn close_provider_wallet(&self, owner: Pubkey, rent_payer: Pubkey) -> Instruction {
        self.ix(
            "close_provider_wallet",
            [],
            vec![
                s(owner),
                w(address::provider_wallet(&self.program, &self.market, &owner).0),
                w(rent_payer),
            ],
        )
    }
    pub fn book_scoped(
        &self,
        m: ModelKeys,
        u: UserKeys,
        booking: Pubkey,
        payer: Pubkey,
        request: &BookRequest,
        mode: BookingMode,
    ) -> Result<Instruction> {
        ensure(
            address::model(&self.program, &self.market, &request.model_id).0 == m.model,
            "booking model id",
        )?;
        let mut data = request.encode()?;
        data.extend(generation_payload(match mode {
            BookingMode::OneShot => None,
            BookingMode::Reusable { generation, .. } => Some(generation),
        })?);
        data.push(u8::from(request.minimum_wallet_capacity.is_some()));
        if let Some(minimum) = request.minimum_wallet_capacity {
            data.extend(minimum.to_le_bytes());
        }
        Ok(self.ix(
            "reserve_list_booking",
            data,
            vec![
                r(self.market),
                r(m.model),
                r(address::list_terms(&self.program, &request.provider_list, &m.model).0),
                w(address::list_capacity(&self.program, &request.provider_list, &m.model).0),
                w(u.user),
                s(u.owner),
                w(booking),
                r(payer),
                r(request.provider_list),
                r(address::list_routing(&self.program, &request.provider_list).0),
                w(address::provider_wallet(&self.program, &self.market, &u.owner).0),
                r(if m.group == Pubkey::default() {
                    self.program
                } else {
                    m.group
                }),
            ],
        ))
    }
}
