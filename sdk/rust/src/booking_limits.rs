use crate::{
    Pubkey, Result,
    accounts::{RawAccount, n32, state},
    ensure,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingLimits {
    pub wallet_window_seconds: u32,
    pub wallet_bookings_per_window: u32,
    pub wallet_booking_capacity: u32,
}
impl Default for BookingLimits {
    fn default() -> Self {
        Self {
            wallet_window_seconds: 60,
            wallet_bookings_per_window: 0,
            wallet_booking_capacity: 0,
        }
    }
}
impl BookingLimits {
    pub fn encode(self) -> Result<Vec<u8>> {
        ensure(
            (1..=86_400).contains(&self.wallet_window_seconds),
            "booking window seconds",
        )?;
        Ok([
            self.wallet_window_seconds,
            self.wallet_bookings_per_window,
            self.wallet_booking_capacity,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect())
    }
}
pub(crate) fn slab<'a>(
    account: &'a RawAccount,
    program: &Pubkey,
    magic: &[u8; 8],
    header: usize,
    stride: usize,
) -> Result<(&'a [u8], u32, usize)> {
    let size = account.data.len();
    ensure(
        size >= header && size <= 10_485_760 && (size - header).is_multiple_of(stride),
        "booking slab size",
    )?;
    let data = state(account, program, magic, size)?;
    let capacity = (size - header) / stride;
    let count = n32(data, header - 4) as usize;
    ensure(capacity > 0 && count <= capacity, "booking slab count")?;
    Ok((data, capacity as u32, count))
}
