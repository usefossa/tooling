use super::*;

pub(super) async fn provider(rt: &Runtime, command: ProviderCommand) -> Result<Value> {
    let wallet = rt.wallet()?;
    let address = address::provider(&rt.protocol.program, &rt.protocol.market, &wallet).0;
    if matches!(command, ProviderCommand::Show) {
        if rt.market.provider_limits_enabled {
            return Ok(
                json!({"address":address.to_string(),"provider":rt.rpc.account::<Provider>(address,rt.protocol.program).await?,"provider_limits":routing::provider_limits_state(rt,wallet).await?}),
            );
        }
        if rt.market.list_routing_enabled {
            return Ok(
                json!({"address":address.to_string(),"provider":rt.rpc.account::<Provider>(address,rt.protocol.program).await?,"provider_policy":rt.rpc.account::<fossa_sdk::scoped::ProviderPolicy>(address::provider_policy(&rt.protocol.program,&rt.protocol.market,&wallet).0,rt.protocol.program).await?}),
            );
        }
        return Ok(
            json!({"address":address.to_string(),"provider":rt.rpc.account::<Provider>(address,rt.protocol.program).await?}),
        );
    }
    let state = rt
        .rpc
        .account::<Provider>(address, rt.protocol.program)
        .await?;
    match command {
        ProviderCommand::Limits {
            concurrency,
            quota,
            quota_window_seconds,
            wallet_concurrency,
            wallet_quota,
            wallet_quota_window_seconds,
            page,
        } => {
            use fossa_sdk::provider_limits::{ProviderLimits, RequestLimits};
            state.ok_or_else(|| Error::Input("Provider missing; use provider register".into()))?;
            routing::configure_provider_limits(
                rt,
                ProviderLimits {
                    global: RequestLimits {
                        concurrency,
                        quota,
                        quota_window_seconds,
                    },
                    wallet: RequestLimits {
                        concurrency: wallet_concurrency,
                        quota: wallet_quota,
                        quota_window_seconds: wallet_quota_window_seconds,
                    },
                },
                page,
            )
            .await
        }
        ProviderCommand::BookingLimits {
            window_seconds,
            bookings_per_window,
            outstanding_bookings,
        } => {
            state.ok_or_else(|| Error::Input("provider missing; use provider register".into()))?;
            routing::configure_policy(
                rt,
                BookingLimits {
                    wallet_window_seconds: window_seconds,
                    wallet_bookings_per_window: bookings_per_window,
                    wallet_booking_capacity: outstanding_bookings,
                },
            )
            .await
        }
        ProviderCommand::Register(args) => {
            if state.is_some() {
                return Err(Error::Input(
                    "provider already registered; use provider update".into(),
                ));
            }
            let doc = document(MetadataKind::Provider, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt
                .protocol
                .initialize_provider(wallet, rt.payer()?, &reference)?;
            rt.transact(
                "register provider",
                vec![ix],
                &[],
                json!({"provider":address.to_string(),"metadata":reference_view(&reference)}),
            )
            .await
        }
        ProviderCommand::Update(args) => {
            let state = state
                .ok_or_else(|| Error::Input("provider missing; use provider register".into()))?;
            let doc = if let Some(path) = &args.metadata_file {
                document(MetadataKind::Provider, path)?
            } else {
                let current = Document::fetch(Kind::Provider, &state.document).await?;
                let image_data = args.image_file.as_deref().map(image).transpose()?;
                let image_uri = image_data
                    .as_ref()
                    .map(|(_, bytes)| {
                        format!(
                            "{}/v1/providers/images/{}",
                            rt.config.api_url,
                            fossa_sdk::metadata::sha256_hex(bytes)
                        )
                    })
                    .or_else(|| args.image_url.clone());
                let document = patch_profile(&current, args.name.as_deref(), image_uri.as_deref())?;
                if rt.options.execute
                    && let Some((mime, bytes)) = &image_data
                {
                    rt.ensure_no_pending()?;
                    runtime::hosted(&rt.config)?
                        .upload_image(mime, bytes)
                        .await?;
                }
                document
            };
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt.protocol.update_provider_metadata(wallet, &reference)?;
            rt.transact("update provider metadata",vec![ix],&[],json!({"provider":address.to_string(),"previous":reference_view(&state.document),"metadata":reference_view(&reference),"document":doc.value(),"image_upload":args.image_file.is_some()})).await
        }
        ProviderCommand::Show => unreachable!(),
    }
}
pub fn patch_profile(
    current: &Document,
    name: Option<&str>,
    image: Option<&str>,
) -> Result<Document> {
    let mut value = current.value().clone();
    if let Some(name) = name {
        value["name"] = name.into();
    }
    if let Some(image) = image {
        value["image"] = image.into();
    }
    Ok(Document::parse(
        Kind::Provider,
        serde_json::to_vec(&value)?,
    )?)
}
