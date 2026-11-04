use super::*;

pub(super) async fn provider_list(rt: &Runtime, command: ListCommand) -> Result<Value> {
    const COMPUTE_UNITS: u32 = 250_000;
    if let ListCommand::Create {
        capacity,
        fee_bps,
        protocol_share_bps,
        maximum_payment,
        metadata_file,
        metadata_uri,
    } = command
    {
        let list = Keypair::new();
        let authority = rt.wallet()?;
        let address = list.pubkey();
        let market = rt
            .rpc
            .require_account::<Market>(rt.protocol.market, rt.protocol.program)
            .await?;
        if !market.creation_enabled
            || market.creation_net_price == 0
            || market.creation_mint == Pubkey::default()
        {
            return Err(Error::Input("List creation is disabled until governance-token payment is configured and enabled".into()));
        }
        let mut instructions = vec![
            fossa_sdk::transaction::compute_unit_limit(COMPUTE_UNITS),
            rt.protocol.initialize_list(
                list.pubkey(),
                authority,
                instruction::ListCreation {
                    limit: capacity,
                    fee_bps,
                    protocol_share: protocol_share_bps,
                    maximum_payment,
                    source: address::associated_token_with_program(
                        &market.creation_mint,
                        &authority,
                        &market.creation_token_program,
                    ),
                    destination: address::associated_token_with_program(
                        &market.creation_mint,
                        &market.council,
                        &market.creation_token_program,
                    ),
                    mint: market.creation_mint,
                    token_program: market.creation_token_program,
                },
            )?,
        ];
        let metadata = if let Some(path) = metadata_file {
            let doc = document(MetadataKind::List, &path)?;
            let reference = publish(rt, &doc, metadata_uri.as_deref()).await?;
            instructions.push(rt.protocol.initialize_list_metadata(
                address,
                authority,
                rt.payer()?,
                &reference,
            )?);
            Some(json!({"reference":reference_view(&reference),"document":doc.value()}))
        } else {
            None
        };
        return rt.transact("create provider list", instructions, &[list], json!({"list":address.to_string(),"authority":authority.to_string(),"capacity":capacity,"metadata":metadata})).await;
    }
    let address = match command {
        ListCommand::Show { list, .. }
        | ListCommand::Update { list, .. }
        | ListCommand::ConfigureFees { list, .. }
        | ListCommand::Add { list, .. }
        | ListCommand::Remove { list, .. }
        | ListCommand::Close { list } => list,
        _ => unreachable!(),
    };
    let state = rt
        .rpc
        .require_account::<ProviderList>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("provider list market mismatch".into()));
    }
    if let ListCommand::Show { fetch_metadata, .. } = &command {
        let metadata = list_metadata(rt, address, &state).await?;
        let document = if *fetch_metadata && let Some(metadata) = &metadata {
            Some(
                Document::fetch(Kind::List, &metadata.document)
                    .await?
                    .value()
                    .clone(),
            )
        } else {
            None
        };
        return Ok(
            json!({"address":address.to_string(),"state":state,"metadata":metadata,"document":document}),
        );
    }
    let authority = rt.wallet()?;
    if state.authority != authority {
        return Err(Error::Input(
            "provider list belongs to another owner".into(),
        ));
    }
    if let ListCommand::Update { profile, .. } = &command {
        return update_list_profile(rt, address, &state, profile).await;
    }
    if let ListCommand::ConfigureFees {
        fee_bps,
        protocol_share_bps,
        ..
    } = command
    {
        let fees = fossa_sdk::reserve::ListFees {
            fee_bps,
            protocol_share: protocol_share_bps,
        };
        return rt.transact("configure list fees", vec![rt.protocol.configure_list_fees(address, authority, fees)?], &[], json!({"list":address.to_string(),"previous":{"fee_bps":state.fee_bps,"protocol_share":state.protocol_share},"next":fees})).await;
    }
    let (label, ix) = match command {
        ListCommand::Add { provider, .. } => {
            if state.contains(&provider) || state.providers.len() == state.limit as usize {
                return Err(Error::Input("provider already listed or list full".into()));
            }
            (
                "add listed provider",
                if state.routing_enabled {
                    rt.protocol
                        .update_routing_list(address, authority, provider, true, false)
                } else {
                    rt.protocol.update_list(address, authority, provider, true)
                },
            )
        }
        ListCommand::Remove { provider, .. } => {
            if !state.contains(&provider) {
                return Err(Error::Input("provider is not listed".into()));
            }
            (
                "remove listed provider",
                if state.routing_enabled {
                    let policy = rt
                        .rpc
                        .account::<fossa_sdk::scoped::ProviderPolicy>(
                            address::provider_policy(
                                &rt.protocol.program,
                                &rt.protocol.market,
                                &provider,
                            )
                            .0,
                            rt.protocol.program,
                        )
                        .await?;
                    rt.protocol.update_routing_list(
                        address,
                        authority,
                        provider,
                        false,
                        policy.is_some(),
                    )
                } else {
                    rt.protocol.update_list(address, authority, provider, false)
                },
            )
        }
        ListCommand::Close { .. } => {
            if !state.providers.is_empty() {
                return Err(Error::Input("provider list must be empty".into()));
            }
            (
                "close provider list",
                rt.protocol.close_list(address, authority),
            )
        }
        _ => unreachable!(),
    };
    let instructions = if matches!(command, ListCommand::Close { .. }) {
        if state.routing_enabled {
            vec![rt.protocol.close_list_routing(address, authority), ix]
        } else {
            vec![ix]
        }
    } else {
        vec![
            fossa_sdk::transaction::compute_unit_limit(COMPUTE_UNITS),
            ix,
        ]
    };
    rt.transact(
        label,
        instructions,
        &[],
        json!({"list":address.to_string()}),
    )
    .await
}

pub(super) async fn list_metadata(
    rt: &Runtime,
    list: Pubkey,
    state: &ProviderList,
) -> Result<Option<ListMetadata>> {
    let address = address::list_metadata(&rt.protocol.program, &list).0;
    let Some(raw) = rt.rpc.raw_account(address).await? else {
        return Ok(None);
    };
    if raw.owner == address::SYSTEM_PROGRAM && raw.data.is_empty() {
        return Ok(None);
    }
    let metadata = ListMetadata::decode(&address, &raw, &rt.protocol.program)?;
    if metadata.market != state.market
        || metadata.list != list
        || metadata.authority != state.authority
    {
        return Err(Error::Input("list metadata binding mismatch".into()));
    }
    Ok(Some(metadata))
}

pub(super) fn patch_list_profile(
    current: &Value,
    patch: &ListProfileArgs,
    image: Option<&str>,
    icon: Option<&str>,
) -> Result<Document> {
    let mut value = current.clone();
    for (field, next) in [
        ("name", patch.name.as_deref()),
        ("accentColor", patch.accent_color.as_deref()),
        ("website", patch.website.as_deref()),
        ("image", image.or(patch.image_url.as_deref())),
        ("icon", icon.or(patch.icon_url.as_deref())),
    ] {
        if let Some(next) = next {
            value[field] = next.into();
        }
    }
    for (field, clear) in [
        ("image", patch.remove_image),
        ("website", patch.remove_website),
        ("icon", patch.remove_icon),
    ] {
        if clear {
            value[field] = Value::Null;
        }
    }
    Ok(Document::parse(Kind::List, serde_json::to_vec(&value)?)?)
}

pub(super) async fn update_list_profile(
    rt: &Runtime,
    list: Pubkey,
    state: &ProviderList,
    args: &ListProfileArgs,
) -> Result<Value> {
    let previous = list_metadata(rt, list, state).await?;
    let doc = if let Some(path) = &args.metadata_file {
        document(MetadataKind::List, path)?
    } else {
        let current = if let Some(metadata) = &previous {
            Document::fetch(Kind::List, &metadata.document)
                .await?
                .value()
                .clone()
        } else {
            json!({"schemaVersion":1})
        };
        let image_data = args.image_file.as_deref().map(image).transpose()?;
        let icon_data = args.icon_file.as_deref().map(icon).transpose()?;
        let asset_uri = |bytes: &[u8]| {
            format!(
                "{}/v1/providers/images/{}",
                rt.config.api_url.trim_end_matches('/'),
                fossa_sdk::metadata::sha256_hex(bytes)
            )
        };
        let image_uri = image_data.as_ref().map(|(_, bytes)| asset_uri(bytes));
        let icon_uri = icon_data.as_deref().map(asset_uri);
        let doc = patch_list_profile(&current, args, image_uri.as_deref(), icon_uri.as_deref())?;
        if rt.options.execute {
            rt.ensure_no_pending()?;
            if let Some((mime, bytes)) = &image_data {
                runtime::hosted(&rt.config)?
                    .upload_image(mime, bytes)
                    .await?;
            }
            if let Some(bytes) = &icon_data {
                runtime::hosted(&rt.config)?
                    .upload_image("image/svg+xml", bytes)
                    .await?;
            }
        }
        doc
    };
    let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
    let instruction = if previous.is_some() {
        rt.protocol
            .update_list_metadata(list, state.authority, &reference)?
    } else {
        rt.protocol
            .initialize_list_metadata(list, state.authority, rt.payer()?, &reference)?
    };
    rt.transact("update list metadata", vec![instruction], &[], json!({"list":list.to_string(),"previous":previous.map(|m|reference_view(&m.document)),"metadata":reference_view(&reference),"document":doc.value(),"image_upload":args.image_file.is_some(),"icon_upload":args.icon_file.is_some()})).await
}
