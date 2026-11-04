use super::*;

pub(super) async fn model(rt: &Runtime, command: ModelCommand) -> Result<Value> {
    match command {
        ModelCommand::AssignGroup { model, group } => {
            model_markets::assign(rt, model, Some(group)).await
        }
        ModelCommand::ClearGroup(model) => model_markets::assign(rt, model, None).await,
        ModelCommand::Show(args) => {
            let address = model_address(rt, &args.model)?;
            let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            let metadata = rt
                .rpc
                .account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            Ok(
                json!({"address":address.to_string(),"model":model,"metadata":metadata,"terms":terms,"capacity":capacity}),
            )
        }
        ModelCommand::Grow { model: args, limit } => {
            let address = model_address(rt, &args.model)?;
            let model = rt
                .rpc
                .require_account::<Model>(address, rt.protocol.program)
                .await?;
            if model.market != rt.protocol.market
                || limit <= model.limit
                || limit - model.limit > 75
            {
                return Err(Error::Input(
                    "new limit must add 1 to 75 slots in this market".into(),
                ));
            }
            let books = rt
                .rpc
                .multiple_accounts(&[model.terms, model.capacity])
                .await?;
            let terms_raw = books.value[0]
                .as_ref()
                .ok_or(fossa_sdk::Error::MissingAccount(model.terms))?;
            let capacity_raw = books.value[1]
                .as_ref()
                .ok_or(fossa_sdk::Error::MissingAccount(model.capacity))?;
            let terms = TermsBook::decode(&model.terms, terms_raw, &rt.protocol.program)?;
            let capacity =
                CapacityBook::decode(&model.capacity, capacity_raw, &rt.protocol.program)?;
            terms.validate_pair(&capacity, &address, &model)?;
            let [terms_size, capacity_size] = instruction::book_sizes(limit)?;
            let (terms_rent, capacity_rent) =
                tokio::try_join!(rt.rpc.rent(terms_size), rt.rpc.rent(capacity_size))?;
            let rent = terms_rent
                .saturating_sub(terms_raw.lamports)
                .checked_add(capacity_rent.saturating_sub(capacity_raw.lamports))
                .ok_or(fossa_sdk::Error::Arithmetic)?;
            rt.transact("grow model books", vec![rt.protocol.grow_model(ModelKeys::from_account(address, &model), rt.payer()?, limit)?], &[], json!({"model":args.model,"previous_limit":model.limit,"limit":limit,"additional_rent_lamports":rent.to_string()})).await
        }
        ModelCommand::UpdateMetadata {
            model,
            document: args,
        } => {
            let address = model_address(rt, &model.model)?;
            let metadata = rt
                .rpc
                .require_account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            if metadata.market != rt.protocol.market
                || metadata.model != address
                || metadata.authority != rt.wallet()?
            {
                return Err(Error::Input(
                    "model metadata authority or binding mismatch".into(),
                ));
            }
            let doc = model_document(&model.model, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            rt.transact(
                "update model metadata",
                vec![
                    rt.protocol
                        .update_model_metadata(address, rt.wallet()?, &reference)?,
                ],
                &[],
                json!({"model":model.model,"metadata":reference_view(&reference)}),
            )
            .await
        }
    }
}
pub fn patch_terms(mut terms: OfferTerms, patch: &OfferPatch) -> Result<OfferTerms> {
    if let Some(value) = patch.checkpoint {
        terms.checkpoint = value;
    }
    if let Some(value) = patch.input_modalities {
        terms.input_modalities = value;
    }
    if let Some(value) = patch.output_modalities {
        terms.output_modalities = value;
    }
    if let Some(value) = patch.context_tokens {
        terms.context_window = value;
    }
    if let Some(value) = patch.input_bound_tokens {
        terms.max_input = value;
    }
    if let Some(value) = patch.output_tokens {
        terms.max_output = value;
    }
    if let Some(n) = patch.input_price {
        terms.rates.input = n;
    }
    if let Some(n) = patch.cached_price {
        terms.rates.cached = n;
    }
    if let Some(n) = patch.output_price {
        terms.rates.output = n;
    }
    if let Some(n) = patch.capacity {
        terms.capacity = n;
    }
    if let Some(n) = patch.speed_tps {
        terms.speed_tps = n;
    }
    if let Some(v) = patch.active {
        terms.active = v;
    }
    if let Some(v) = patch.private {
        terms.private = v;
    }
    terms.validate()?;
    Ok(terms)
}
pub(super) async fn selected_offer(
    rt: &Runtime,
    args: &OfferArg,
) -> Result<(OfferKeys, Offer, Capacity)> {
    offer_at(rt, model_address(rt, &args.model.model)?, args.slot).await
}
pub(super) async fn offer_at(
    rt: &Runtime,
    address: Pubkey,
    slot: Option<u32>,
) -> Result<(OfferKeys, Offer, Capacity)> {
    let (model, terms, capacities) = rt.rpc.model_books(&rt.protocol, address).await?;
    let wallet = rt.wallet()?;
    let slot = if let Some(n) = slot {
        n
    } else {
        let matches = terms
            .entries
            .iter()
            .enumerate()
            .filter(|(_, offer)| offer.provider == wallet)
            .map(|(i, _)| i as u32)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error::Input(
                "select --slot: wallet must have exactly one offer to infer slot".into(),
            ));
        }
        matches[0]
    };
    let offer = terms
        .entries
        .get(slot as usize)
        .ok_or_else(|| Error::Input("offer slot out of range".into()))?
        .clone();
    if offer.provider != wallet {
        return Err(Error::Input("offer belongs to another provider".into()));
    }
    let keys = OfferKeys {
        model: ModelKeys::from_account(address, &model),
        provider: wallet,
        payout: offer.payout,
        slot,
    };
    Ok((keys, offer, capacities.entries[slot as usize]))
}
pub(super) async fn offer(rt: &Runtime, command: OfferCommand) -> Result<Value> {
    if rt.market.list_routing_enabled {
        return Err(Error::Input("List routing is active; use routing show --model or routing offer for the selected list".into()));
    }
    match command {
        OfferCommand::List(args) => {
            let address = model_address(rt, &args.model)?;
            let (_, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            Ok(
                json!({"model":args.model,"address":address.to_string(),"rate_unit":"micro-USDC per 1,000,000 tokens","offers":terms.entries,"capacity":capacity.entries}),
            )
        }
        OfferCommand::Update { offer, patch } => {
            let (keys, current, _) = selected_offer(rt, &offer).await?;
            let next = patch_terms(current.terms, &patch)?;
            rt.transact("update offer",vec![rt.protocol.update_offer(keys,next)?],&[],json!({"model":offer.model.model,"slot":keys.slot,"previous":current.terms,"next":next,"rate_unit":"micro-USDC per 1,000,000 tokens"})).await
        }
    }
}
