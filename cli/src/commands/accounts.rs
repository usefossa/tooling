use super::*;

pub(super) async fn validate_token(rt: &Runtime, account: Pubkey, owner: Pubkey) -> Result<()> {
    let raw = rt
        .rpc
        .raw_account(account)
        .await?
        .ok_or(fossa_sdk::Error::MissingAccount(account))?;
    TokenAccount::decode(&raw)?.validate(&rt.protocol.mint, &owner)?;
    Ok(())
}
pub(super) async fn account(rt: &Runtime, address: Pubkey, kind: AccountKind) -> Result<Value> {
    macro_rules! typed {
        ($t:ty) => {
            serde_json::to_value(
                rt.rpc
                    .require_account::<$t>(address, rt.protocol.program)
                    .await?,
            )?
        };
    }
    let state = match kind {
        AccountKind::Market => typed!(Market),
        AccountKind::Provider => typed!(Provider),
        AccountKind::Model => typed!(Model),
        AccountKind::ModelMetadata => typed!(ModelMetadata),
        AccountKind::ModelGroup => typed!(ModelGroup),
        AccountKind::User => typed!(User),
        AccountKind::Terms => typed!(TermsBook),
        AccountKind::Capacity => typed!(CapacityBook),
        AccountKind::Booking => typed!(Booking),
        AccountKind::ProviderList => typed!(ProviderList),
        AccountKind::ListMetadata => typed!(ListMetadata),
        AccountKind::Token => serde_json::to_value(TokenAccount::decode(
            &rt.rpc
                .raw_account(address)
                .await?
                .ok_or(fossa_sdk::Error::MissingAccount(address))?,
        )?)?,
    };
    Ok(json!({"address":address.to_string(),"account":state}))
}
pub(super) async fn transaction(
    config: &Config,
    options: &Options,
    command: &TransactionCommand,
) -> Result<Value> {
    let (path, resume) = match command {
        TransactionCommand::Status { file } => (file, false),
        TransactionCommand::Resume { file } => (file, true),
    };
    let _journal_lock = runtime::JournalLock::acquire(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let pending = PendingTransaction::load(path)?;
    let rpc = runtime::rpc(config)?;
    let signature = pending.validate()?;
    if resume && options.execute {
        rpc.submit(&pending)
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let status = rpc
            .confirm(&pending, Duration::from_secs(options.confirmation_seconds))
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let archive = runtime::archive_pending(path, &pending)?;
        return Ok(
            json!({"mode":"executed","signature":pending.signature,"status":status,"journal":archive}),
        );
    }
    let mut status = rpc.signature_status(&signature).await?;
    let finalized_height = if status.is_none() {
        Some(
            rpc.clone()
                .with_commitment(fossa_sdk::rpc::Commitment::Finalized)
                .block_height()
                .await?,
        )
    } else {
        None
    };
    let expired = finalized_height.is_some_and(|n| n > pending.last_valid_block_height);
    if expired {
        status = rpc.signature_status(&signature).await?;
    }
    Ok(
        json!({"mode":if resume {"plan"} else {"inspection"},"signature":pending.signature,"status":status,"expired_without_observed_status":expired&&status.is_none(),"last_valid_block_height":pending.last_valid_block_height,"journal":path,"next":if resume {"--execute reconciles then resends only original bytes"} else {"use transaction resume --file PATH --execute; preserve unresolved journals"}}),
    )
}
pub(super) async fn user(rt: &Runtime, command: UserCommand) -> Result<Value> {
    if let UserCommand::Show { user } = command {
        return account(rt, user, AccountKind::User).await;
    }
    let wallet = rt.wallet()?;
    if let UserCommand::Init = command {
        let key = Keypair::new();
        let address = key.pubkey();
        let authority = address::vault(&rt.protocol.program, &address).0;
        let vault = address::associated_token(&rt.protocol.mint, &authority);
        let payer = rt.payer()?;
        let ix = vec![
            instruction::create_associated_token(payer, authority, rt.protocol.mint),
            rt.protocol.initialize_user(UserKeys {
                user: address,
                owner: wallet,
                vault,
            }),
        ];
        return rt
            .transact(
                "initialize user",
                ix,
                &[key],
                json!({"user":address.to_string(),"vault":vault.to_string()}),
            )
            .await;
    }
    let (address, amount, token, deposit) = match command {
        UserCommand::Deposit {
            user,
            amount,
            source,
        } => (user, amount, source, true),
        UserCommand::Withdraw {
            user,
            amount,
            destination,
        } => (user, amount, destination, false),
        _ => unreachable!(),
    };
    let state = rt
        .rpc
        .require_account::<User>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market || state.owner != wallet {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let default_token = token.is_none();
    let token = token.unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &wallet));
    let keys = UserKeys::from_account(address, &state);
    let ix = if deposit {
        validate_token(rt, token, wallet).await?;
        vec![rt.protocol.deposit(keys, token, amount)?]
    } else {
        let mut instructions = Vec::with_capacity(2);
        if default_token {
            instructions.push(instruction::create_associated_token(
                rt.payer()?,
                wallet,
                rt.protocol.mint,
            ));
        }
        instructions.push(rt.protocol.withdraw(keys, token, amount)?);
        instructions
    };
    rt.transact(if deposit {"deposit user funds"} else {"withdraw user funds"},ix,&[],json!({"user":address.to_string(),"amount_atomic":amount.to_string(),"amount_usdc":amount::format_usdc(amount)})).await
}
