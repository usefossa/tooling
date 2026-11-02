use crate::Pubkey;
use sha2::{Digest, Sha256};

pub const SYSTEM_PROGRAM: Pubkey = Pubkey::new_from_array([0; 32]);
pub const TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const NATIVE_MINT: Pubkey =
    Pubkey::from_str_const("So11111111111111111111111111111111111111112");
pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn model_id(id: &str) -> [u8; 32] {
    Sha256::digest(id.as_bytes()).into()
}
pub fn model_group(program: &Pubkey, market: &Pubkey, id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"model_group", market.as_ref(), id], program)
}
pub fn reserve_mint(program: &Pubkey, group: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"reserve_mint", group.as_ref()], program)
}
pub fn model(program: &Pubkey, market: &Pubkey, id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"model", market.as_ref(), id], program)
}
pub fn provider(program: &Pubkey, market: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"provider", market.as_ref(), authority.as_ref()], program)
}
pub fn model_metadata(program: &Pubkey, model: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"model_metadata", model.as_ref()], program)
}
pub fn list_metadata(program: &Pubkey, list: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"list_metadata", list.as_ref()], program)
}
pub fn vault(program: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", user.as_ref()], program)
}
pub fn associated_token(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    associated_token_with_program(mint, owner, &TOKEN_PROGRAM)
}
pub fn associated_token_with_program(mint: &Pubkey, owner: &Pubkey, token: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM,
    )
    .0
}

pub fn provider_policy(program: &Pubkey, market: &Pubkey, provider: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"provider_policy", market.as_ref(), provider.as_ref()],
        program,
    )
}
pub fn provider_capacity(program: &Pubkey, model: &Pubkey, provider: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"provider_capacity", model.as_ref(), provider.as_ref()],
        program,
    )
}
pub fn list_routing(program: &Pubkey, list: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"list_routing", list.as_ref()], program)
}
pub fn list_terms(program: &Pubkey, list: &Pubkey, model: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"list_terms", list.as_ref(), model.as_ref()], program)
}
pub fn list_capacity(program: &Pubkey, list: &Pubkey, model: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"list_capacity", list.as_ref(), model.as_ref()], program)
}
pub fn provider_wallet(program: &Pubkey, market: &Pubkey, owner: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"provider_wallet", market.as_ref(), owner.as_ref()],
        program,
    )
}
