use crate::{Pubkey, Result, ensure};

pub fn global(program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"protocol"], program)
}
pub fn config(program: &Pubkey, quote_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config", quote_mint.as_ref()], program)
}
pub fn pool(program: &Pubkey, mint_a: &Pubkey, mint_b: &Pubkey) -> Result<(Pubkey, u8)> {
    ensure(mint_a != mint_b, "distinct pool mints")?;
    let (first, last) = if mint_a.as_ref() < mint_b.as_ref() {
        (mint_a, mint_b)
    } else {
        (mint_b, mint_a)
    };
    Ok(Pubkey::find_program_address(
        &[b"pool", first.as_ref(), last.as_ref()],
        program,
    ))
}
pub fn authority(program: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"authority", pool.as_ref()], program)
}
pub fn lp_mint(program: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"lp_mint", pool.as_ref()], program)
}
