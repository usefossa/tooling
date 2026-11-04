use super::*;

pub(super) async fn booking(rt: &Runtime, command: BookingCommand) -> Result<Value> {
    if let BookingCommand::Create(args) = command {
        return book(rt, args).await;
    }
    let address = match command {
        BookingCommand::Show { booking }
        | BookingCommand::Claim { booking, .. }
        | BookingCommand::Release { booking }
        | BookingCommand::Expire { booking } => booking,
        BookingCommand::Create(_) => unreachable!(),
    };
    if matches!(command, BookingCommand::Show { .. }) {
        return account(rt, address, AccountKind::Booking).await;
    }
    let state = rt
        .rpc
        .require_account::<Booking>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("booking market mismatch".into()));
    }
    let model = rt
        .rpc
        .require_account::<Model>(state.model, rt.protocol.program)
        .await?;
    let model = ModelKeys::from_account(state.model, &model);
    let user = rt
        .rpc
        .require_account::<User>(state.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market {
        return Err(Error::Input("booking user market mismatch".into()));
    }
    let keys = SettlementKeys::from_booking(
        address,
        &state,
        model,
        UserKeys::from_account(state.user, &user),
        &rt.protocol.mint,
    )?;
    let generation = (state.generation > 0).then_some(state.generation);
    let (label, ix) = match command {
        BookingCommand::Claim {
            uncached,
            cached,
            output,
            ..
        } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            if u64::from(uncached) + u64::from(cached) > u64::from(state.max_input)
                || output > state.max_output
            {
                return Err(Error::Input("usage exceeds booking token caps".into()));
            }
            let usage = Usage {
                uncached,
                cached,
                output,
            };
            let receipt = generation
                .map(|_| fossa_sdk::receipt::Receipt::from_booking(address, &state, usage))
                .transpose()?;
            (
                "claim booking",
                rt.protocol.claim(keys, usage, receipt.as_ref())?,
            )
        }
        BookingCommand::Release { .. } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            ("release booking", rt.protocol.release(keys, generation)?)
        }
        BookingCommand::Expire { .. } => ("expire booking", rt.protocol.expire(keys, generation)?),
        _ => unreachable!(),
    };
    rt.transact(
        label,
        vec![ix],
        &[],
        json!({"booking":address.to_string(),"generation":state.generation.to_string()}),
    )
    .await
}

pub(super) fn book_request(args: &BookArgs, default_list: Pubkey) -> instruction::BookRequest {
    instruction::BookRequest {
        minimum_wallet_capacity: args.minimum_wallet_capacity,
        min_context: args.min_context,
        limits_configured: args.limits_configured || args.min_context > 0,
        checkpoint: args.checkpoint,
        input_modalities: args.input_modalities,
        output_modalities: args.output_modalities,
        model_id: address::model_id(&args.model.model),
        provider_list: args.list.unwrap_or(default_list),
        rate_caps: Rates {
            input: args.input_price_cap,
            cached: args.cached_price_cap,
            output: args.output_price_cap,
        },
        min_speed: args.min_speed,
        max_input: args.max_input,
        max_output: args.max_output,
        private: args.private,
        max_spend: args.max_spend,
        estimated_input: args.estimated_input.unwrap_or(args.max_input),
        estimated_cached: args.estimated_cached,
        estimated_output: args.estimated_output.unwrap_or(args.max_output),
        cache_provider: args.cache_provider.unwrap_or_default(),
    }
}
pub(super) async fn book(rt: &Runtime, args: BookArgs) -> Result<Value> {
    if rt.market.list_routing_enabled {
        return routing::book(rt, args).await;
    }
    let user = rt
        .rpc
        .require_account::<User>(args.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market || user.owner != rt.wallet()? {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let address = model_address(rt, &args.model.model)?;
    let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
    let list = rt
        .rpc
        .require_account::<ProviderList>(
            args.list.unwrap_or(rt.config.provider_list),
            rt.protocol.program,
        )
        .await?;
    if list.market != rt.protocol.market {
        return Err(Error::Input("provider list market mismatch".into()));
    }
    let request = book_request(&args, rt.config.provider_list);
    let selected = request
        .select(&terms, &capacity, &list, user.free)?
        .ok_or_else(|| {
            Error::Input(
                "no eligible offer fits list, price, speed, privacy, capacity and balance limits"
                    .into(),
            )
        })?;
    let key = Keypair::new();
    let booking = key.pubkey();
    let payer = rt.payer()?;
    let ix = vec![
        fossa_sdk::transaction::compute_unit_limit(
            (20_000 + 500 * terms.entries.len().min(4096) as u32).min(1_400_000),
        ),
        rt.protocol.initialize_reusable_booking(
            UserKeys::from_account(args.user, &user),
            booking,
            payer,
        ),
        rt.protocol.book(
            ModelKeys::from_account(address, &model),
            UserKeys::from_account(args.user, &user),
            booking,
            payer,
            &request,
            instruction::BookingMode::OneShot,
        )?,
    ];

    rt.transact("create booking",ix,&[key],json!({"booking":booking.to_string(),"user":args.user.to_string(),"candidate_slot":selected.slot,"reserve_atomic":selected.reserve.to_string(),"selection":"on-chain selection uses execution-time state"})).await
}
