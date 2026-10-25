use crate::{
    Instruction, Pubkey, Result,
    accounts::{Booking, BookingStatus, i64le, key, n64},
    amount::Usage,
    ensure,
};

pub const RECEIPT_LEN: usize = 120;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub owner: Pubkey,
    pub user: Pubkey,
    pub booking: Pubkey,
    pub generation: u64,
    pub charge: u64,
    pub claim_end: i64,
}
impl Receipt {
    pub fn encode(&self) -> [u8; RECEIPT_LEN] {
        let mut p = [0; RECEIPT_LEN];
        p[..32].copy_from_slice(self.owner.as_ref());
        p[32..64].copy_from_slice(self.user.as_ref());
        p[64..96].copy_from_slice(self.booking.as_ref());
        p[96..104].copy_from_slice(&self.generation.to_le_bytes());
        p[104..112].copy_from_slice(&self.charge.to_le_bytes());
        p[112..].copy_from_slice(&self.claim_end.to_le_bytes());
        p
    }
    pub fn decode(p: &[u8]) -> Result<Self> {
        ensure(p.len() == RECEIPT_LEN, "receipt length")?;
        Ok(Self {
            owner: key(p, 0),
            user: key(p, 32),
            booking: key(p, 64),
            generation: n64(p, 96),
            charge: n64(p, 104),
            claim_end: i64le(p, 112),
        })
    }
    pub fn from_booking(address: Pubkey, booking: &Booking, usage: Usage) -> Result<Self> {
        ensure(
            booking.status == BookingStatus::Reserved && booking.generation > 0,
            "reusable reserved booking",
        )?;
        ensure(
            u64::from(usage.uncached) + u64::from(usage.cached) <= u64::from(booking.max_input)
                && usage.output <= booking.max_output
                && (usage.cached == 0 || booking.cache_allowed),
            "claim usage",
        )?;
        let charge = booking.rates.charge(usage);
        ensure(charge <= booking.reserve, "claim charge")?;
        Ok(Self {
            owner: booking.owner,
            user: booking.user,
            booking: address,
            generation: booking.generation,
            charge,
            claim_end: booking.claim_end,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedReceipt {
    pub receipt: Receipt,
    pub usage: Usage,
}
impl PublishedReceipt {
    pub fn from_claim(program: &Pubkey, ix: &Instruction) -> Result<Self> {
        ensure(
            ix.program_id == *program
                && ix.data.len() == 141
                && ix.data[0] == 7
                && ix.accounts.len() == 11,
            "published claim instruction",
        )?;
        let p = &ix.data[1..];
        let receipt = Receipt::decode(&p[20..])?;
        ensure(
            receipt.generation == n64(p, 12)
                && receipt.booking == ix.accounts[4].pubkey
                && receipt.user == ix.accounts[3].pubkey,
            "published receipt bindings",
        )?;
        Ok(Self {
            receipt,
            usage: Usage {
                uncached: crate::accounts::n32(p, 0),
                cached: crate::accounts::n32(p, 4),
                output: crate::accounts::n32(p, 8),
            },
        })
    }
}
