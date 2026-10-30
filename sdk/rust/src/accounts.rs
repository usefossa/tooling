use crate::{
    Pubkey, Result, address,
    amount::{Rates, Usage},
    ensure,
};
use serde::{Deserialize, Serialize};

pub const MARKET_LEN: usize = 248;
pub const MODEL_LEN: usize = 176;
pub const PROVIDER_LEN: usize = 364;
pub const MODEL_METADATA_LEN: usize = 396;
pub const LIST_METADATA_LEN: usize = 396;
pub const USER_LEN: usize = 128;
pub const BOOKING_LEN: usize = 472;
pub const HEADER_LEN: usize = 48;
pub const TERMS_LEN: usize = 120;
pub const CAPACITY_LEN: usize = 32;
pub const MAX_ENTRIES: u32 = 4096;
pub const LIST_HEADER_LEN: usize = 88;
pub const LIST_SLOT_LEN: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawAccount {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    pub lamports: u64,
    pub executable: bool,
    pub data: Vec<u8>,
}

pub trait Decode: Sized {
    const MAGIC: &'static [u8; 8];
    fn decode(address: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self>;
}

pub(crate) fn state<'a>(
    account: &'a RawAccount,
    program: &Pubkey,
    magic: &[u8; 8],
    len: usize,
) -> Result<&'a [u8]> {
    ensure(
        account.owner == *program
            && !account.executable
            && account.data.len() == len
            && account.data.get(..8) == Some(magic),
        "account owner, size or discriminator",
    )?;
    Ok(&account.data)
}
pub(crate) fn key(d: &[u8], at: usize) -> Pubkey {
    Pubkey::new_from_array(d[at..at + 32].try_into().unwrap())
}
pub(crate) fn n16(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().unwrap())
}
pub(crate) fn n32(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().unwrap())
}
pub(crate) fn n64(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}
pub(crate) fn i64le(d: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}
pub(crate) fn flag(d: &[u8], at: usize) -> Result<bool> {
    ensure(d[at] <= 1, "boolean flag")?;
    Ok(d[at] == 1)
}
fn rates(d: &[u8], at: usize) -> Rates {
    Rates {
        input: n32(d, at),
        cached: n32(d, at + 4),
        output: n32(d, at + 8),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRef {
    pub sha256: [u8; 32],
    pub uri: String,
}
impl DocumentRef {
    pub fn new(sha256: [u8; 32], uri: impl Into<String>) -> Result<Self> {
        let uri = uri.into();
        ensure(
            !uri.is_empty() && uri.len() <= 256 && !uri.bytes().any(|b| b <= 32 || b == 127),
            "metadata URI",
        )?;
        Ok(Self { sha256, uri })
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        Self::new(self.sha256, self.uri.clone())?;
        Ok([
            self.sha256.as_slice(),
            &(self.uri.len() as u32).to_le_bytes(),
            self.uri.as_bytes(),
        ]
        .concat())
    }
    fn decode(d: &[u8], hash: usize, uri: usize) -> Result<Self> {
        let len = usize::from(n16(d, hash + 32));
        ensure((1..=256).contains(&len), "metadata URI length")?;
        ensure(
            d[uri + len..].iter().all(|&b| b == 0),
            "metadata URI padding",
        )?;
        let uri = std::str::from_utf8(&d[uri..uri + len])
            .map_err(|_| crate::Error::Invalid("metadata URI UTF-8"))?;
        Self::new(d[hash..hash + 32].try_into().unwrap(), uri)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Market {
    #[serde(with = "crate::serde_pubkey")]
    pub council: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub bank: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub creation_mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub creation_token_program: Pubkey,
    pub creation_net_price: u64,
    pub creation_enabled: bool,
    #[serde(with = "crate::serde_pubkey")]
    pub dex_program: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub dex_config: Pubkey,
}
impl Decode for Market {
    const MAGIC: &'static [u8; 8] = &[219, 190, 213, 55, 0, 227, 198, 154];
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MARKET_LEN)?;
        Ok(Self {
            council: key(d, 8),
            mint: key(d, 40),
            bank: key(d, 72),
            creation_mint: key(d, 104),
            creation_token_program: key(d, 136),
            creation_net_price: n64(d, 168),
            creation_enabled: flag(d, 176)?,
            dex_program: key(d, 184),
            dex_config: key(d, 216),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Model {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub id: [u8; 32],
    #[serde(with = "crate::serde_pubkey")]
    pub terms: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub capacity: Pubkey,
    pub limit: u32,
    pub bump: u8,
    #[serde(with = "crate::serde_pubkey")]
    pub group: Pubkey,
}
impl Decode for Model {
    const MAGIC: &'static [u8; 8] = &[152, 221, 247, 122, 185, 125, 223, 151];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MODEL_LEN)?;
        let value = Self {
            market: key(d, 8),
            id: d[40..72].try_into().unwrap(),
            terms: key(d, 72),
            capacity: key(d, 104),
            limit: n32(d, 136),
            bump: d[140],
            group: key(d, 144),
        };
        ensure(
            (1..=MAX_ENTRIES).contains(&value.limit) && value.terms != value.capacity,
            "model book bindings",
        )?;
        ensure(
            address::model(p, &value.market, &value.id) == (*at, value.bump),
            "model PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Provider {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub document: DocumentRef,
    pub bump: u8,
}
impl Decode for Provider {
    const MAGIC: &'static [u8; 8] = &[164, 180, 71, 17, 75, 216, 80, 195];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, PROVIDER_LEN)?;
        let value = Self {
            market: key(d, 8),
            authority: key(d, 40),
            document: DocumentRef::decode(d, 72, 108)?,
            bump: d[107],
        };
        ensure(
            address::provider(p, &value.market, &value.authority) == (*at, value.bump),
            "provider PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelMetadata {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub document: DocumentRef,
    pub bump: u8,
}
impl Decode for ModelMetadata {
    const MAGIC: &'static [u8; 8] = &[243, 161, 2, 132, 123, 88, 51, 61];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MODEL_METADATA_LEN)?;
        let value = Self {
            market: key(d, 8),
            model: key(d, 40),
            authority: key(d, 72),
            document: DocumentRef::decode(d, 104, 140)?,
            bump: d[138],
        };
        ensure(
            address::model_metadata(p, &value.model) == (*at, value.bump),
            "model metadata PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct User {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub free: u64,
    #[serde(with = "crate::serde_pubkey")]
    pub vault: Pubkey,
    pub bump: u8,
    pub generation: u64,
}
impl Decode for User {
    const MAGIC: &'static [u8; 8] = &[159, 117, 95, 227, 239, 151, 58, 236];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, USER_LEN)?;
        let value = Self {
            owner: key(d, 8),
            market: key(d, 40),
            free: n64(d, 72),
            vault: key(d, 80),
            bump: d[120],
            generation: n64(d, 112),
        };
        ensure(address::vault(p, at).1 == value.bump, "user vault PDA bump")?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferTerms {
    pub rates: Rates,
    pub speed_tps: u16,
    pub private: bool,
    pub active: bool,
    pub capacity: u32,
}
impl OfferTerms {
    pub fn validate(self) -> Result<()> {
        self.rates.validate()?;
        ensure(
            self.speed_tps > 0 && self.capacity > 0,
            "offer speed or capacity",
        )
    }
    pub fn encode(self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::with_capacity(20);
        self.rates.encode(&mut out);
        out.extend(self.speed_tps.to_le_bytes());
        out.extend([u8::from(self.private), u8::from(self.active)]);
        out.extend(self.capacity.to_le_bytes());
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub payout: Pubkey,
    pub terms: OfferTerms,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Capacity {
    pub active: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TermsBook {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    pub limit: u32,
    pub entries: Vec<Offer>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapacityBook {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    pub limit: u32,
    pub entries: Vec<Capacity>,
}
fn book<'a>(
    a: &'a RawAccount,
    p: &Pubkey,
    magic: &[u8; 8],
    size: usize,
) -> Result<(&'a [u8], Pubkey, u32, usize)> {
    ensure(a.data.len() >= HEADER_LEN, "book header")?;
    let limit = n32(&a.data, 40);
    let count = n32(&a.data, 44);
    ensure(
        (1..=MAX_ENTRIES).contains(&limit) && count <= limit,
        "book limit or count",
    )?;
    let d = state(a, p, magic, HEADER_LEN + limit as usize * size)?;
    Ok((d, key(d, 8), limit, count as usize))
}
impl Decode for TermsBook {
    const MAGIC: &'static [u8; 8] = &[223, 24, 40, 223, 249, 219, 14, 97];
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, model, limit, count) = book(a, p, Self::MAGIC, TERMS_LEN)?;
        let mut entries = Vec::with_capacity(count);
        for t in d[48..48 + count * TERMS_LEN]
            .as_chunks::<TERMS_LEN>()
            .0
            .iter()
        {
            let terms = OfferTerms {
                rates: rates(t, 96),
                speed_tps: n16(t, 108),
                private: flag(t, 110)?,
                active: flag(t, 111)?,
                capacity: n32(t, 116),
            };
            terms.validate()?;
            entries.push(Offer {
                provider: key(t, 0),
                payout: key(t, 32),
                terms,
            });
        }
        Ok(Self {
            model,
            limit,
            entries,
        })
    }
}
impl Decode for CapacityBook {
    const MAGIC: &'static [u8; 8] = &[19, 216, 196, 142, 39, 29, 145, 16];
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, model, limit, count) = book(a, p, Self::MAGIC, CAPACITY_LEN)?;
        let entries = d[48..48 + count * CAPACITY_LEN]
            .as_chunks::<CAPACITY_LEN>()
            .0
            .iter()
            .map(|c| Capacity { active: n32(c, 16) })
            .collect();
        Ok(Self {
            model,
            limit,
            entries,
        })
    }
}
impl TermsBook {
    pub fn validate_pair(&self, capacity: &CapacityBook, at: &Pubkey, model: &Model) -> Result<()> {
        ensure(
            self.model == *at
                && capacity.model == *at
                && self.limit == model.limit
                && capacity.limit == model.limit
                && self.entries.len() == capacity.entries.len(),
            "model offer books",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum BookingStatus {
    Reserved = 0,
    Released = 3,
    Finalized = 4,
    Available = 5,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Booking {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub reserve: u64,
    pub rates: Rates,
    pub max_input: u32,
    pub max_output: u32,
    pub claim_end: i64,
    pub status: BookingStatus,
    pub generation: u64,
    pub charge: u64,
    pub usage: Usage,
    pub min_speed: u16,
    #[serde(with = "crate::serde_pubkey")]
    pub user: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub rent_payer: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub payout: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider_list: Pubkey,
    pub slot: u32,
    pub requested_privacy: bool,
    pub cache_allowed: bool,
    pub estimated_score: u64,
    #[serde(with = "crate::serde_pubkey")]
    pub creator: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub protocol_destination: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub reserve_authority: Pubkey,
    pub fee_bps: u16,
    pub protocol_share: u16,
    pub reserve_destination: bool,
}
impl Decode for Booking {
    const MAGIC: &'static [u8; 8] = &[147, 50, 61, 138, 208, 21, 254, 156];
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, BOOKING_LEN)?;
        let status = match d[148] {
            0 => BookingStatus::Reserved,
            3 => BookingStatus::Released,
            4 => BookingStatus::Finalized,
            5 => BookingStatus::Available,
            _ => return Err(crate::Error::Invalid("booking status")),
        };
        ensure(d[359] == 0, "booking reserved marker")?;
        Ok(Self {
            owner: key(d, 8),
            provider: key(d, 40),
            market: key(d, 72),
            reserve: n64(d, 104),
            rates: rates(d, 112),
            max_input: n32(d, 124),
            max_output: n32(d, 128),
            claim_end: i64le(d, 132),
            status,
            generation: n64(d, 152),
            charge: n64(d, 160),
            usage: Usage {
                uncached: n32(d, 168),
                cached: n32(d, 172),
                output: n32(d, 176),
            },
            min_speed: n16(d, 180),
            user: key(d, 192),
            model: key(d, 224),
            rent_payer: key(d, 256),
            payout: key(d, 288),
            provider_list: key(d, 320),
            slot: n32(d, 352),
            requested_privacy: flag(d, 357)?,
            cache_allowed: flag(d, 358)?,
            estimated_score: n64(d, 360),
            creator: key(d, 368),
            protocol_destination: key(d, 400),
            reserve_authority: key(d, 432),
            fee_bps: n16(d, 464),
            protocol_share: n16(d, 466),
            reserve_destination: flag(d, 468)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderList {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub limit: u32,
    pub fee_bps: u16,
    pub protocol_share: u16,
    #[serde(serialize_with = "provider_keys")]
    pub providers: Vec<Pubkey>,
}
impl ProviderList {
    pub fn contains(&self, provider: &Pubkey) -> bool {
        self.providers
            .binary_search_by(|key| key.as_ref().cmp(provider.as_ref()))
            .is_ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListMetadata {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub list: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub document: DocumentRef,
    pub bump: u8,
}
impl Decode for ListMetadata {
    const MAGIC: &'static [u8; 8] = &[245, 77, 238, 20, 114, 17, 94, 226];
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, LIST_METADATA_LEN)?;
        let value = Self {
            market: key(d, 8),
            list: key(d, 40),
            authority: key(d, 72),
            document: DocumentRef::decode(d, 104, 140)?,
            bump: d[138],
        };
        ensure(
            d[139] == 0 && address::list_metadata(p, &value.list) == (*at, value.bump),
            "list metadata PDA or reserved byte",
        )?;
        Ok(value)
    }
}
impl Decode for ProviderList {
    const MAGIC: &'static [u8; 8] = &[169, 24, 186, 110, 22, 139, 190, 82];
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        ensure(a.data.len() >= LIST_HEADER_LEN, "provider list header")?;
        let limit = n32(&a.data, 72);
        let count = n32(&a.data, 84);
        ensure(
            (1..=MAX_ENTRIES).contains(&limit) && count <= limit,
            "provider list capacity",
        )?;
        let d = state(
            a,
            p,
            Self::MAGIC,
            LIST_HEADER_LEN + limit as usize * LIST_SLOT_LEN,
        )?;
        ensure(
            d[LIST_HEADER_LEN + count as usize * 32..]
                .iter()
                .all(|&b| b == 0),
            "provider list unused slots",
        )?;
        let providers: Vec<_> = d[LIST_HEADER_LEN..LIST_HEADER_LEN + count as usize * 32]
            .as_chunks::<32>()
            .0
            .iter()
            .map(|key| Pubkey::new_from_array(*key))
            .collect();
        ensure(
            providers
                .windows(2)
                .all(|keys| keys[0].as_ref() < keys[1].as_ref()),
            "sorted unique providers",
        )?;
        Ok(Self {
            market: key(d, 8),
            authority: key(d, 40),
            limit,
            fee_bps: n16(d, 76),
            protocol_share: n16(d, 78),
            providers,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenAccount {
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    pub amount: u64,
    pub frozen: bool,
}
impl TokenAccount {
    pub fn decode(a: &RawAccount) -> Result<Self> {
        ensure(
            a.owner == address::TOKEN_PROGRAM && !a.executable && a.data.len() == 165,
            "SPL token account",
        )?;
        let d = &a.data;
        ensure(d[108] == 1 || d[108] == 2, "initialized token account")?;
        Ok(Self {
            mint: key(d, 0),
            owner: key(d, 32),
            amount: n64(d, 64),
            frozen: d[108] == 2,
        })
    }
    pub fn validate(&self, mint: &Pubkey, owner: &Pubkey) -> Result<()> {
        ensure(
            self.mint == *mint && self.owner == *owner && !self.frozen,
            "token mint, authority or frozen state",
        )
    }
}

fn provider_keys<S: serde::Serializer>(
    keys: &[Pubkey],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    keys.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .serialize(serializer)
}

impl Booking {
    pub fn fee_destinations(&self, mint: &Pubkey) -> Option<[Pubkey; 2]> {
        (self.fee_bps > 0).then(|| {
            [
                address::associated_token(mint, &self.creator),
                if self.reserve_destination {
                    self.protocol_destination
                } else {
                    address::associated_token(mint, &self.protocol_destination)
                },
            ]
        })
    }
}

pub const MODEL_GROUP_LEN: usize = 144;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelGroup {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub group_id: [u8; 32],
    #[serde(with = "crate::serde_pubkey")]
    pub backing_vault: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub fixed_token_mint: Pubkey,
    pub route_to_reserve: bool,
    pub bump: u8,
}
impl Decode for ModelGroup {
    const MAGIC: &'static [u8; 8] = &[237, 175, 79, 136, 212, 228, 240, 177];
    fn decode(at: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self> {
        let d = state(account, program, Self::MAGIC, MODEL_GROUP_LEN)?;
        let value = Self {
            market: key(d, 8),
            group_id: d[40..72].try_into().unwrap(),
            backing_vault: key(d, 72),
            fixed_token_mint: key(d, 104),
            route_to_reserve: flag(d, 136)?,
            bump: d[137],
        };
        ensure(
            address::model_group(program, &value.market, &value.group_id) == (*at, value.bump)
                && address::reserve_mint(program, at).0 == value.fixed_token_mint
                && d[138..].iter().all(|&b| b == 0),
            "model group PDA, mint or reserved bytes",
        )?;
        Ok(value)
    }
}
