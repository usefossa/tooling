use std::{collections::HashMap, path::PathBuf, sync::atomic::Ordering};

use fossa_jupiter::{FossaAmm, PROGRAM_ID, swap_instruction_data};
use jupiter_amm_interface::{
    Amm, AmmContext, ClockRef, FlowType, KeyedAccount, Quote, QuoteParams, SwapMode, SwapParams,
};
use jupiter_amm_test_kit::{PoolSnapshot, PoolTest, assert_pool_parity};
use solana_account::Account;
use solana_pubkey::{Pubkey, pubkey};

const MAINNET_POOL: Pubkey = pubkey!("24dwAL1KYHMtDdd4UJD2KchL2xVBTxyz6YbwhNrQwPDJ");
const CLOCK: Pubkey = pubkey!("SysvarC1ock11111111111111111111111111111111");

struct Fixture {
    amm: FossaAmm,
    snapshot: PoolSnapshot,
    accounts: HashMap<Pubkey, Box<Account>>,
    clock: ClockRef,
    dir: PathBuf,
}

fn fixture(name: &str) -> Fixture {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic.json")).unwrap();
    let (key, dir) = if name == "mainnet" {
        (
            MAINNET_POOL,
            PathBuf::from("tests/fixtures/accounts").join(MAINNET_POOL.to_string()),
        )
    } else {
        let case = if name == "amm_epoch_after" {
            "amm_epoch_before"
        } else {
            name
        };
        let key: Pubkey = cases[case].as_str().unwrap().parse().unwrap();
        (
            key,
            PathBuf::from("tests/fixtures/accounts")
                .join(name)
                .join(key.to_string()),
        )
    };
    let snapshot = PoolSnapshot::load_dir(&dir).unwrap();
    let clock = ClockRef::from(snapshot.clock().unwrap());
    let mut amm = FossaAmm::from_keyed_account(
        &KeyedAccount {
            key,
            account: snapshot.get(&key).unwrap().clone(),
            params: None,
        },
        &AmmContext {
            clock_ref: clock.clone(),
        },
    )
    .unwrap();
    let accounts = amm
        .get_accounts_to_update()
        .into_iter()
        .map(|key| (key, Box::new(snapshot.get(&key).unwrap().clone())))
        .collect();
    amm.update(&accounts).unwrap();
    Fixture {
        amm,
        snapshot,
        accounts,
        clock,
        dir,
    }
}

fn params(amm: &FossaAmm, a_to_b: bool, amount: u64, mode: SwapMode) -> QuoteParams {
    let mints = amm.get_reserve_mints();
    QuoteParams {
        amount,
        input_mint: mints[usize::from(!a_to_b)],
        output_mint: mints[usize::from(a_to_b)],
        swap_mode: mode,
        flow_type: FlowType::Binary,
    }
}

fn parity(name: &str, amounts: &[u64]) {
    let f = fixture(name);
    let mints = f.amm.get_reserve_mints();
    let mut test = PoolTest::new(f.amm.key());
    test.fixtures_dir = f.dir;
    for &amount in amounts {
        test = test
            .add_swap(mints[0], mints[1], amount)
            .add_swap(mints[1], mints[0], amount);
    }
    assert_pool_parity::<FossaAmm>(&test, |swap, amount| {
        swap_instruction_data(swap, amount, 0).unwrap()
    });
}

#[test]
fn mainnet_quotes_match_captured_deployed_program() {
    parity("mainnet", &[1_000_000, 10_000_000]);
}

#[test]
fn deployed_program_fixture_matches_recorded_digest() {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/program.json")).unwrap();
    let bytes = std::fs::read("tests/fixtures/fossa.so").unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        manifest["sha256"].as_str().unwrap()
    );
    assert_eq!(
        PROGRAM_ID.to_string(),
        manifest["programId"].as_str().unwrap()
    );
    assert_eq!(MAINNET_POOL.to_string(), manifest["pool"].as_str().unwrap());
}

#[test]
fn amm_quotes_match_execution_with_legacy_tokens_and_transfer_fees() {
    for name in [
        "amm_legacy",
        "amm_taxed_base",
        "amm_taxed_both",
        "amm_graduated",
    ] {
        parity(name, &[1_000, 10_000, 1_000_000]);
    }
}

#[test]
fn curves_quote_virtual_reserves_and_separate_creator_fee_rounding() {
    for name in ["curve_model", "curve_standard"] {
        parity(name, &[10_000, 1_000_000, 1_000_000_000]);
    }
}

#[test]
fn transfer_fee_epoch_changes_match_execution() {
    for name in ["amm_epoch_before", "amm_epoch_after"] {
        parity(name, &[1_000, 1_000_000]);
    }
    let f = fixture("amm_epoch_before");
    let before = f
        .amm
        .quote(&params(&f.amm, true, 1_000_000, SwapMode::ExactIn))
        .unwrap();
    f.clock.epoch.store(1, Ordering::Relaxed);
    let after = f
        .amm
        .quote(&params(&f.amm, true, 1_000_000, SwapMode::ExactIn))
        .unwrap();
    assert_ne!(before.out_amount, after.out_amount);
    let next = fixture("amm_epoch_after");
    assert_eq!(
        after.out_amount,
        next.amm
            .quote(&params(&next.amm, true, 1_000_000, SwapMode::ExactIn))
            .unwrap()
            .out_amount
    );
}

#[test]
fn invalid_updates_clear_previously_valid_quotes() {
    let f = fixture("amm_legacy");
    let quote = params(&f.amm, true, 10_000, SwapMode::ExactIn);
    for missing in f.amm.get_accounts_to_update() {
        let mut amm = f.amm.clone();
        let mut accounts = f.accounts.clone();
        accounts.remove(&missing);
        assert!(amm.update(&accounts).is_err());
        assert!(amm.quote(&quote).is_err());
        assert!(!amm.is_active());
        amm.update(&f.accounts).unwrap();
        assert!(amm.quote(&quote).is_ok());
    }
    for address in f.amm.get_accounts_to_update() {
        let mut amm = f.amm.clone();
        let mut accounts = f.accounts.clone();
        accounts.get_mut(&address).unwrap().owner = Pubkey::new_unique();
        assert!(amm.update(&accounts).is_err());
        assert!(amm.quote(&quote).is_err());
    }
    let mut accounts = f.accounts.clone();
    for address in f.amm.get_accounts_to_update().into_iter().skip(3) {
        accounts.get_mut(&address).unwrap().data[64..72].copy_from_slice(&0u64.to_le_bytes());
    }
    let mut amm = f.amm.clone();
    assert!(amm.update(&accounts).is_err());
}

#[test]
fn curve_capacity_mint_pairs_and_clock_fail_closed() {
    let f = fixture("curve_model");
    assert!(f.amm.is_active());
    assert!(
        f.amm
            .quote(&params(&f.amm, false, u64::MAX, SwapMode::ExactIn))
            .is_err()
    );
    assert!(
        f.amm
            .quote(&params(&f.amm, false, 0, SwapMode::ExactIn))
            .is_err()
    );
    let mut invalid = params(&f.amm, false, 1_000_000, SwapMode::ExactIn);
    invalid.output_mint = invalid.input_mint;
    assert!(f.amm.quote(&invalid).is_err());
    f.clock.unix_timestamp.store(-1, Ordering::Relaxed);
    assert!(
        f.amm
            .quote(&params(&f.amm, false, 1_000_000, SwapMode::ExactIn))
            .is_err()
    );
    let empty = fixture("curve_empty");
    assert!(empty.amm.is_active());
    assert!(
        empty
            .amm
            .quote(&params(&empty.amm, false, 1_000_000, SwapMode::ExactIn))
            .is_ok()
    );
    assert!(
        empty
            .amm
            .quote(&params(&empty.amm, true, 1_000_000, SwapMode::ExactIn))
            .is_err()
    );
    let complete = fixture("curve_complete");
    assert!(!complete.amm.is_active());
    assert!(
        complete
            .amm
            .quote(&params(&complete.amm, false, 1_000_000, SwapMode::ExactIn))
            .is_err()
    );
}

#[test]
fn instruction_metas_use_transfer_authority_and_stored_token_order() {
    let f = fixture("amm_taxed_both");
    let authority = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let jupiter = Pubkey::new_unique();
    for direction in [true, false] {
        for mode in [SwapMode::ExactIn, SwapMode::ExactOut] {
            let quote = params(&f.amm, direction, 1_000_000, mode);
            let sam = f
                .amm
                .get_swap_and_account_metas(&SwapParams {
                    swap_mode: mode,
                    in_amount: 1_000_000,
                    out_amount: 1_000,
                    source_mint: quote.input_mint,
                    destination_mint: quote.output_mint,
                    source_token_account: source,
                    destination_token_account: destination,
                    token_transfer_authority: authority,
                    user: Pubkey::new_unique(),
                    payer: Pubkey::new_unique(),
                    quote_mint_to_referrer: None,
                    jupiter_program_id: &jupiter,
                    missing_dynamic_accounts_as_default: false,
                })
                .unwrap();
            assert_eq!(sam.account_metas.len(), f.amm.get_accounts_len());
            assert_eq!(sam.account_metas[0].pubkey, authority);
            assert!(sam.account_metas[0].is_signer);
            assert_eq!(
                sam.account_metas
                    .iter()
                    .filter(|meta| meta.is_signer)
                    .count(),
                1
            );
            assert_eq!(
                sam.account_metas[7].pubkey,
                if direction { source } else { destination }
            );
            assert_eq!(
                sam.account_metas[8].pubkey,
                if direction { destination } else { source }
            );
            let data = swap_instruction_data(&sam.swap, 42, 7).unwrap();
            assert_eq!(data[8], u8::from(direction));
            assert_eq!(&data[9..17], &42u64.to_le_bytes());
            assert_eq!(&data[17..25], &7u64.to_le_bytes());
        }
    }
}

#[test]
fn exact_output_quotes_match_both_native_token_deltas() {
    use litesvm::LiteSVM;
    use solana_keypair::Keypair;
    use solana_signer::Signer;
    use solana_transaction::Transaction;
    use spl_token_2022_interface::{
        extension::StateWithExtensionsMut, state::Account as TokenAccount,
    };

    for name in [
        "mainnet",
        "amm_legacy",
        "amm_taxed_both",
        "amm_epoch_after",
        "curve_standard",
    ] {
        for direction in [true, false] {
            let f = fixture(name);
            let request = params(&f.amm, direction, 10_000, SwapMode::ExactOut);
            let quote: Quote = f.amm.quote(&request).unwrap();
            let mut svm = LiteSVM::new();
            svm.add_program(
                PROGRAM_ID,
                &std::fs::read("tests/fixtures/fossa.so").unwrap(),
            )
            .unwrap();
            for (&key, account) in &f.accounts {
                svm.set_account(key, (**account).clone()).unwrap();
            }
            svm.set_account(CLOCK, f.snapshot.get(&CLOCK).unwrap().clone())
                .unwrap();
            let user = Keypair::new();
            let user_key = user.pubkey();
            svm.airdrop(&user_key, 10_000_000_000).unwrap();
            let mut token_accounts = Vec::new();
            for mint in [request.input_mint, request.output_mint] {
                let program = f.snapshot.get(&mint).unwrap().owner;
                let create =
                    fossa_sdk::token::create_associated_token(user_key, user_key, mint, program)
                        .unwrap();
                let address = create.accounts[1].pubkey;
                let tx = Transaction::new_signed_with_payer(
                    &[create],
                    Some(&user_key),
                    &[&user],
                    svm.latest_blockhash(),
                );
                svm.send_transaction(tx).unwrap();
                token_accounts.push(address);
            }
            let mut source = svm.get_account(&token_accounts[0]).unwrap();
            let mut state =
                StateWithExtensionsMut::<TokenAccount>::unpack(&mut source.data).unwrap();
            state.base.amount = quote.in_amount;
            state.pack_base();
            svm.set_account(token_accounts[0], source).unwrap();
            let sam = f
                .amm
                .get_swap_and_account_metas(&SwapParams {
                    swap_mode: SwapMode::ExactOut,
                    in_amount: quote.in_amount,
                    out_amount: quote.out_amount,
                    source_mint: request.input_mint,
                    destination_mint: request.output_mint,
                    source_token_account: token_accounts[0],
                    destination_token_account: token_accounts[1],
                    token_transfer_authority: user_key,
                    user: user_key,
                    payer: user_key,
                    quote_mint_to_referrer: None,
                    jupiter_program_id: &PROGRAM_ID,
                    missing_dynamic_accounts_as_default: false,
                })
                .unwrap();
            let swap = solana_instruction::Instruction {
                program_id: PROGRAM_ID,
                accounts: sam.account_metas,
                data: swap_instruction_data(&sam.swap, quote.out_amount, quote.in_amount).unwrap(),
            };
            svm.send_transaction(Transaction::new_signed_with_payer(
                &[swap],
                Some(&user_key),
                &[&user],
                svm.latest_blockhash(),
            ))
            .unwrap();
            let balance = |key| {
                fossa_sdk::token::TokenAccount::decode(&fossa_sdk::accounts::RawAccount {
                    owner: svm.get_account(&key).unwrap().owner,
                    lamports: 1,
                    executable: false,
                    data: svm.get_account(&key).unwrap().data,
                })
                .unwrap()
                .amount
            };
            assert_eq!(balance(token_accounts[0]), 0, "{name} input delta");
            assert_eq!(
                balance(token_accounts[1]),
                quote.out_amount,
                "{name} output delta"
            );
        }
    }
}
