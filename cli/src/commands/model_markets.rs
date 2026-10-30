use super::*;
use fossa_sdk::{
    dex::{Dex, accounts as dex_accounts},
    reserve::{
        FeeConfig, GroupInit, GroupKeys, InitializeModelGroup, Redeem, RedemptionKeys,
        ReserveConfig, TokenMetadata,
    },
    token::{self, TokenMint},
};

pub(super) fn preflight(command: &Command) -> Result<()> {
    match command {
        Command::Group(GroupCommand::Create {
            accounts_file,
            parameters_file,
            ..
        }) => {
            let _: InitializeModelGroup =
                serde_json::from_slice(&read_limited(accounts_file, 16_384)?)?;
            let config: GroupInit = serde_json::from_slice(&read_limited(parameters_file, 4096)?)?;
            config.metadata.validate()?;
            if config.tax_bps > 10_000 {
                return Err(Error::Input(
                    "transfer tax exceeds 10000 basis points".into(),
                ));
            }
        }
        Command::Group(GroupCommand::UpdateMetadata {
            name, symbol, uri, ..
        }) => {
            TokenMetadata {
                name: name.clone(),
                symbol: symbol.clone(),
                uri: uri.clone(),
            }
            .validate()?;
        }
        Command::Council(CouncilCommand::ConfigureFees {
            bank: None,
            creation_mint: None,
            creation_token_program: None,
            creation_net_price: None,
            creation_enabled: None,
        }) => {
            return Err(Error::Input("specify at least one fee-policy field".into()));
        }
        _ => (),
    }
    Ok(())
}

async fn market(rt: &Runtime, authorized: bool) -> Result<Market> {
    let state = rt
        .rpc
        .require_account::<Market>(rt.protocol.market, rt.protocol.program)
        .await?;
    if authorized && state.council != rt.wallet()? {
        return Err(Error::Input("wallet is not router council".into()));
    }
    Ok(state)
}

async fn raw(rt: &Runtime, address: Pubkey) -> Result<RawAccount> {
    Ok(rt
        .rpc
        .raw_account(address)
        .await?
        .ok_or(fossa_sdk::Error::MissingAccount(address))?)
}

async fn group_state(rt: &Runtime, group: Pubkey) -> Result<(GroupKeys, ModelGroup, TokenMint)> {
    let state = rt
        .rpc
        .require_account::<ModelGroup>(group, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("model group belongs to another market".into()));
    }
    let keys = GroupKeys::from_account(group, &state);
    let mint = TokenMint::decode(&raw(rt, keys.mint).await?)?;
    if mint.program != address::TOKEN_2022_PROGRAM
        || mint.mint_authority.is_some()
        || mint.freeze_authority.is_some()
        || mint.transfer_fee_authority != Some(group)
        || mint.withdraw_withheld_authority != Some(group)
    {
        return Err(Error::Input("model-token authority mismatch".into()));
    }
    Ok((keys, state, mint))
}

async fn token_account(
    rt: &Runtime,
    account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    program: Pubkey,
    vault: bool,
) -> Result<token::TokenAccount> {
    let state = token::TokenAccount::decode(&raw(rt, account).await?)?;
    state.validate(mint, owner, vault)?;
    if state.program != program {
        return Err(Error::Input("token program mismatch".into()));
    }
    Ok(state)
}

pub(super) async fn assign(rt: &Runtime, model: ModelArg, group: Option<Pubkey>) -> Result<Value> {
    market(rt, true).await?;
    let model_address = super::model_address(rt, &model.model)?;
    let state = rt
        .rpc
        .require_account::<Model>(model_address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("model market mismatch".into()));
    }
    if let Some(address) = group {
        let state = rt
            .rpc
            .require_account::<ModelGroup>(address, rt.protocol.program)
            .await?;
        if state.market != rt.protocol.market {
            return Err(Error::Input("model group belongs to another market".into()));
        }
    }
    rt.transact("assign model group", vec![rt.protocol.assign_model_group(rt.wallet()?, model_address, group)], &[],
        json!({"model":model.model,"previous_group":state.group.to_string(),"group":group.map(|key|key.to_string()),"applies_to":"future bookings"})).await
}

pub(super) async fn group(rt: &Runtime, command: GroupCommand) -> Result<Value> {
    if let GroupCommand::Create {
        accounts_file,
        parameters_file,
        backing_keypair,
        curve_lp_keypair,
    } = command
    {
        return create(
            rt,
            &accounts_file,
            &parameters_file,
            backing_keypair.as_deref(),
            curve_lp_keypair.as_deref(),
        )
        .await;
    }
    let group = match &command {
        GroupCommand::Show { group }
        | GroupCommand::UpdateMetadata { group, .. }
        | GroupCommand::ConfigureReserve { group, .. }
        | GroupCommand::Redeem { group, .. }
        | GroupCommand::CollectTax { group, .. }
        | GroupCommand::Sweep { group, .. } => *group,
        GroupCommand::Create { .. } => unreachable!(),
    };
    let (keys, state, mint) = group_state(rt, group).await?;
    let policy = market(
        rt,
        matches!(
            command,
            GroupCommand::UpdateMetadata { .. } | GroupCommand::ConfigureReserve { .. }
        ),
    )
    .await?;
    match command {
        GroupCommand::Show { .. } => {
            let backing = token_account(
                rt,
                keys.backing,
                rt.protocol.mint,
                group,
                address::TOKEN_PROGRAM,
                true,
            )
            .await?;
            Ok(
                json!({"address":group.to_string(),"group":state,"mint":mint,"backing":backing,"redemption_fee_lamports":fossa_sdk::reserve::REDEMPTION_FEE_LAMPORTS.to_string()}),
            )
        }
        GroupCommand::UpdateMetadata {
            name, symbol, uri, ..
        } => {
            let metadata = TokenMetadata { name, symbol, uri };
            rt.transact(
                "update group metadata",
                vec![rt.protocol.update_group_metadata(
                    rt.wallet()?,
                    rt.payer()?,
                    keys,
                    &metadata,
                )?],
                &[],
                json!({"group":group.to_string(),"metadata":metadata}),
            )
            .await
        }
        GroupCommand::ConfigureReserve {
            route_to_reserve,
            tax_bps,
            maximum_tax,
            ..
        } => {
            if route_to_reserve && mint.supply == 0 {
                return Err(Error::Input(
                    "cannot route fees to zero-supply group".into(),
                ));
            }
            let config = ReserveConfig {
                route_to_reserve,
                tax_bps,
                maximum_tax,
            };
            rt.transact("configure model reserve", vec![rt.protocol.configure_reserve(rt.wallet()?, keys, config)?], &[], json!({"group":group.to_string(),"next":config,"applies_to":"future bookings; token program schedules transfer-tax changes"})).await
        }
        GroupCommand::Redeem {
            amount,
            minimum_output,
            source,
            destination,
            ..
        } => {
            let holder = rt.wallet()?;
            let source = source.unwrap_or_else(|| {
                address::associated_token_with_program(
                    &keys.mint,
                    &holder,
                    &address::TOKEN_2022_PROGRAM,
                )
            });
            let destination = destination
                .unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &holder));
            let balance = token_account(
                rt,
                source,
                keys.mint,
                holder,
                address::TOKEN_2022_PROGRAM,
                false,
            )
            .await?;
            token_account(
                rt,
                destination,
                rt.protocol.mint,
                holder,
                address::TOKEN_PROGRAM,
                false,
            )
            .await?;
            let backing = token_account(
                rt,
                keys.backing,
                rt.protocol.mint,
                group,
                address::TOKEN_PROGRAM,
                true,
            )
            .await?;
            let output =
                fossa_sdk::reserve::redemption_output(backing.amount, mint.supply, amount)?;
            if balance.amount < amount || output < minimum_output {
                return Err(Error::Input(
                    "insufficient tokens or minimum redemption output not met".into(),
                ));
            }
            let ix = rt.protocol.redeem_reserve(
                RedemptionKeys {
                    group: keys,
                    holder,
                    source,
                    destination,
                    council: policy.council,
                },
                Redeem {
                    amount,
                    minimum_output,
                },
            )?;
            rt.transact("redeem model backing", vec![ix], &[], json!({"group":group.to_string(),"burn_atoms":amount.to_string(),"quoted_usdc_atoms":output.to_string(),"minimum_usdc_atoms":minimum_output.to_string(),"fee_lamports":if holder == policy.council {0} else {fossa_sdk::reserve::REDEMPTION_FEE_LAMPORTS}.to_string()})).await
        }
        GroupCommand::CollectTax { destination, .. } => {
            token_account(
                rt,
                destination,
                keys.mint,
                policy.bank,
                address::TOKEN_2022_PROGRAM,
                false,
            )
            .await?;
            rt.transact("collect model transfer tax", vec![rt.protocol.collect_reserve_tax(keys, destination)], &[], json!({"group":group.to_string(),"bank":policy.bank.to_string(),"destination":destination.to_string(),"harvested_mint_tax_atoms":mint.withheld_amount.to_string()})).await
        }
        GroupCommand::Sweep { destination, .. } => {
            if mint.supply != 0 {
                return Err(Error::Input(
                    "backing sweep requires zero token supply".into(),
                ));
            }
            let destination = destination
                .unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &policy.council));
            token_account(
                rt,
                destination,
                rt.protocol.mint,
                policy.council,
                address::TOKEN_PROGRAM,
                false,
            )
            .await?;
            token_account(
                rt,
                keys.backing,
                rt.protocol.mint,
                group,
                address::TOKEN_PROGRAM,
                true,
            )
            .await?;
            rt.transact(
                "sweep empty model backing",
                vec![rt.protocol.sweep_reserve(keys, destination)],
                &[],
                json!({"group":group.to_string(),"destination":destination.to_string()}),
            )
            .await
        }
        GroupCommand::Create { .. } => unreachable!(),
    }
}

async fn dex_state(
    rt: &Runtime,
    dex: Dex,
    council: Pubkey,
) -> Result<(dex_accounts::Global, dex_accounts::Config)> {
    if !raw(rt, dex.program).await?.executable {
        return Err(Error::Input("DEX program is not executable".into()));
    }
    let global = rt
        .rpc
        .require_account::<dex_accounts::Global>(dex.global_address().0, dex.program)
        .await?;
    let config = rt
        .rpc
        .require_account::<dex_accounts::Config>(
            dex.config_address(rt.protocol.mint).0,
            dex.program,
        )
        .await?;
    if global.council != council
        || global.usdc_mint != rt.protocol.mint
        || config.quote_mint != rt.protocol.mint
    {
        return Err(Error::Input("DEX council or USDC binding mismatch".into()));
    }
    Ok((global, config))
}

async fn create(
    rt: &Runtime,
    accounts: &Path,
    parameters: &Path,
    backing_keypair: Option<&Path>,
    lp_keypair: Option<&Path>,
) -> Result<Value> {
    let policy = market(rt, true).await?;
    let keys: InitializeModelGroup = serde_json::from_slice(&read_limited(accounts, 16_384)?)?;
    let config: GroupInit = serde_json::from_slice(&read_limited(parameters, 4096)?)?;
    if keys.council != rt.wallet()?
        || keys.payer != rt.payer()?
        || keys.dex_program != policy.dex_program
        || policy.dex_program == Pubkey::default()
    {
        return Err(Error::Input(
            "prepared council, payer or configured DEX mismatch".into(),
        ));
    }
    let instructions = vec![
        fossa_sdk::transaction::compute_unit_limit(1_000_000),
        rt.protocol.initialize_model_group(keys, &config)?,
    ];
    fossa_sdk::transaction::PreparedTransaction::new(
        &instructions,
        rt.payer()?,
        fossa_sdk::Hash::default(),
        0,
        0,
    )?;
    let dex = Dex {
        program: keys.dex_program,
    };
    let (global, quote) = dex_state(rt, dex, keys.council).await?;
    if keys.dex_treasury != global.treasury
        || !quote.enabled
        || policy.dex_config != dex.config_address(rt.protocol.mint).0
    {
        return Err(Error::Input(
            "DEX treasury or enabled quote configuration mismatch".into(),
        ));
    }
    let group = address::model_group(&rt.protocol.program, &rt.protocol.market, &keys.group_id).0;
    let mint = address::reserve_mint(&rt.protocol.program, &group).0;
    let pool = dex.pool_address(mint, rt.protocol.mint)?.0;
    for account in [
        group,
        mint,
        pool,
        keys.backing,
        keys.curve_lp,
        dex.lp_mint_address(pool).0,
    ] {
        if let Some(raw) = rt.rpc.raw_account(account).await?
            && (raw.owner != address::SYSTEM_PROGRAM || !raw.data.is_empty() || raw.executable)
        {
            return Err(Error::Input(format!(
                "launch account {account} is already initialized"
            )));
        }
    }
    for account in [keys.council_tokens, keys.curve_base_vault] {
        token::validate_uninitialized_fee_account(&raw(rt, account).await?)?;
    }
    token_account(
        rt,
        keys.curve_quote_source,
        rt.protocol.mint,
        keys.council,
        address::TOKEN_PROGRAM,
        false,
    )
    .await?;
    token_account(
        rt,
        keys.curve_quote_vault,
        rt.protocol.mint,
        dex.authority_address(pool).0,
        address::TOKEN_PROGRAM,
        true,
    )
    .await?;
    let mut signers = Vec::new();
    if rt.options.execute {
        for (path, expected) in [(backing_keypair, keys.backing), (lp_keypair, keys.curve_lp)] {
            let signer = fossa_sdk::transaction::read_keypair(path.ok_or_else(|| {
                Error::Input("execution requires backing and curve LP keypairs".into())
            })?)?;
            if signer.pubkey() != expected {
                return Err(Error::Input(
                    "launch cosigner does not match prepared accounts".into(),
                ));
            }
            signers.push(signer);
        }
    }
    rt.transact("create model group", instructions, &signers,
        json!({"group":group.to_string(),"mint":mint.to_string(),"pool":pool.to_string(),"parameters":config,"prepared_accounts":keys,"pool_creation_fee_lamports":fossa_sdk::dex::quote::action_fee(keys.council, keys.dex_treasury).to_string()})).await
}

pub(super) async fn council(rt: &Runtime, command: CouncilCommand) -> Result<Value> {
    let state = market(rt, !matches!(command, CouncilCommand::Show)).await?;
    match command {
        CouncilCommand::Show => Ok(json!({"market":rt.protocol.market.to_string(),"policy":state})),
        CouncilCommand::ConfigureFees {
            bank,
            creation_mint,
            creation_token_program,
            creation_net_price,
            creation_enabled,
        } => {
            let next = FeeConfig {
                bank: bank.unwrap_or(state.bank),
                creation_mint: creation_mint.unwrap_or(state.creation_mint),
                creation_token_program: creation_token_program
                    .unwrap_or(state.creation_token_program),
                creation_net_price: creation_net_price.unwrap_or(state.creation_net_price),
                creation_enabled: creation_enabled.unwrap_or(state.creation_enabled),
            };
            let ix = rt.protocol.configure_fees(state.council, next)?;
            if next.creation_enabled {
                let mint = TokenMint::decode_payment(&raw(rt, next.creation_mint).await?)?;
                if mint.program != next.creation_token_program {
                    return Err(Error::Input("creation mint token program mismatch".into()));
                }
            }
            rt.transact(
                "configure router fees",
                vec![ix],
                &[],
                json!({"previous":state,"next":next}),
            )
            .await
        }
        CouncilCommand::ConfigureDex { program } => {
            let dex = Dex { program };
            dex_state(rt, dex, state.council).await?;
            rt.transact("configure router DEX", vec![rt.protocol.configure_dex(state.council, dex)], &[], json!({"dex_program":program.to_string(),"dex_config":dex.config_address(rt.protocol.mint).0.to_string()})).await
        }
        CouncilCommand::Rotate { next, next_keypair } => {
            if next == Pubkey::default() || next == state.council {
                return Err(Error::Input(
                    "next council must be a different nonzero wallet".into(),
                ));
            }
            let mut signers = Vec::new();
            if rt.options.execute {
                let signer =
                    fossa_sdk::transaction::read_keypair(next_keypair.as_deref().ok_or_else(
                        || Error::Input("execution requires --next-keypair".into()),
                    )?)?;
                if signer.pubkey() != next {
                    return Err(Error::Input(
                        "next keypair does not match next council".into(),
                    ));
                }
                signers.push(signer);
            }
            rt.transact("rotate router council", vec![rt.protocol.rotate_council(state.council, next)], &signers, json!({"previous":state.council.to_string(),"next":next.to_string(),"treasury_rotates":true})).await
        }
    }
}
