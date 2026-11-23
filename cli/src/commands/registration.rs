use super::*;

pub(super) async fn onboard(rt: &Runtime) -> Result<Value> {
    let cfg = &rt.config.file.provider;
    let file = cfg
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    let doc = document(MetadataKind::Provider, &rt.config.base.join(file))?;
    validate_uri(cfg.metadata_uri.as_deref())?;
    let reference = match &cfg.metadata_uri {
        Some(uri) => doc.reference(uri)?,
        None => hosted_reference(&rt.config, &doc)?,
    };
    let mut documents = vec![(doc, cfg.metadata_uri.as_deref())];
    let mut models = Vec::new();
    for model in &cfg.models {
        let document = model_document(&model.id, &rt.config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        let reference = match &model.metadata_uri {
            Some(uri) => document.reference(uri)?,
            None => hosted_reference(&rt.config, &document)?,
        };
        let terms = OfferTerms {
            checkpoint: model_checkpoint(model, &document)?,
            input_modalities: crate::args::parse_modalities(&model.input_modalities)
                .map_err(Error::Input)?,
            output_modalities: crate::args::parse_modalities(&model.output_modalities)
                .map_err(Error::Input)?,
            context_window: model.context_tokens,
            max_input: model.input_bound_tokens,
            max_output: model.output_tokens,
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(cfg.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(cfg.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(cfg.private).unwrap_or(false),
            active: model.active.or(cfg.active).unwrap_or(true),
        };
        terms.validate()?;
        models.push(onboarding::ModelConfig {
            id: model.id.clone(),
            document: reference,
            terms,
            slot: model.slot,
        });
        documents.push((document, model.metadata_uri.as_deref()));
    }
    let config = onboarding::OnboardConfig {
        provider: rt.wallet()?,
        payer: rt.payer()?,
        document: reference,
        book_limit: cfg.model_book_limit.unwrap_or(16),
        models,
    };
    let plan = if rt.market.list_routing_enabled {
        onboarding::plan_for_list(&rt.rpc, &rt.protocol, &config, rt.config.provider_list).await?
    } else {
        onboarding::plan(&rt.rpc, &rt.protocol, &config).await?
    };
    let view = json!({"mode":"plan","operation":"onboard","blockers":plan.blockers,"steps":plan.steps.iter().map(|s|json!({"operation":s.label,"instructions":runtime::instruction_view(&s.instructions)})).collect::<Vec<_>>(),"metadata":documents.iter().map(|(d,_)|document_view(d)).collect::<Vec<_>>()});
    if !rt.options.execute {
        return Ok(view);
    }
    if !plan.blockers.is_empty() {
        return Err(Error::Input(format!(
            "onboarding blocked: {}",
            plan.blockers.join("; ")
        )));
    }
    rt.ensure_no_pending()?;
    for (doc, uri) in &documents {
        publish(rt, doc, *uri).await?;
    }
    let mut completed = Vec::new();
    for step in plan.steps {
        completed.push(
            rt.transact(
                &step.label,
                step.instructions,
                &step.signers,
                json!({"onboarding":true}),
            )
            .await?,
        );
    }
    Ok(json!({"mode":"executed","operation":"onboard","steps":completed}))
}

pub(super) fn model_checkpoint(
    model: &crate::config::Model,
    document: &Document,
) -> Result<[u8; 32]> {
    let checkpoints = document.value()["checkpoints"].as_array();
    let earliest = checkpoints.and_then(|items| {
        items
            .iter()
            .find(|checkpoint| checkpoint["ordered"] != false)
            .or_else(|| items.first())
            .and_then(|checkpoint| checkpoint["id"].as_str())
    });
    let checkpoint = model.checkpoint.as_deref().or(earliest).ok_or_else(|| {
        Error::Input("Model metadata must publish at least one checkpoint identity.".into())
    })?;
    let published: Vec<_> = checkpoints
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str())
        .collect();
    if !published.contains(&checkpoint) {
        return Err(Error::Input(format!(
            "Unknown checkpoint {checkpoint}. Published checkpoints: {}",
            published.join(", ")
        )));
    }
    if model
        .input_modalities
        .split(',')
        .any(|name| name == "image")
    {
        let limit = document.value()["imageTokensByCheckpoint"]
            .get(checkpoint)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                Error::Input(format!(
                    "Image offers require a published image token limit for {checkpoint}."
                ))
            })?;
        if limit > u64::from(model.input_bound_tokens) {
            return Err(Error::Input(
                "Image token limit exceeds the offer input bound.".into(),
            ));
        }
    }
    crate::args::parse_checkpoint(checkpoint).map_err(Error::Input)
}
pub(super) async fn verify_published_offer(
    rt: &Runtime,
    id: &str,
    terms: &OfferTerms,
) -> Result<()> {
    if !terms.active || terms.capacity == 0 {
        return Ok(());
    }
    let model = model_address(rt, id)?;
    let key = address::model_metadata(&rt.protocol.program, &model).0;
    let metadata = rt
        .rpc
        .require_account::<ModelMetadata>(key, rt.protocol.program)
        .await?;
    if metadata.market != rt.protocol.market || metadata.model != model {
        return Err(Error::Input("Model metadata binding mismatch".into()));
    }
    let document = Document::fetch(Kind::Model, &metadata.document).await?;
    let value = document.value();
    if value["id"].as_str() != Some(id) {
        return Err(Error::Input("Model metadata id mismatch".into()));
    }
    let versions: Vec<_> = value["checkpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str())
        .collect();
    let checkpoint = versions
        .iter()
        .copied()
        .find(|id| address::model_id(id) == terms.checkpoint)
        .ok_or_else(|| {
            Error::Input(format!(
                "Unknown checkpoint. Published checkpoints: {}",
                versions.join(", ")
            ))
        })?;
    if terms.input_modalities & 2 != 0
        && !value["imageTokensByCheckpoint"][checkpoint]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= u64::from(terms.max_input))
    {
        return Err(Error::Input(format!(
            "Invalid published image token limit for {checkpoint}."
        )));
    }
    Ok(())
}
pub(super) fn validate_onboard(config: &Config) -> Result<()> {
    let provider = &config.file.provider;
    let path = provider
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    document(MetadataKind::Provider, &config.base.join(path))?;
    validate_uri(provider.metadata_uri.as_deref())?;
    instruction::book_sizes(provider.model_book_limit.unwrap_or(16))?;
    for model in &provider.models {
        let document = model_document(&model.id, &config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        OfferTerms {
            checkpoint: model_checkpoint(model, &document)?,
            input_modalities: crate::args::parse_modalities(&model.input_modalities)
                .map_err(Error::Input)?,
            output_modalities: crate::args::parse_modalities(&model.output_modalities)
                .map_err(Error::Input)?,
            context_window: model.context_tokens,
            max_input: model.input_bound_tokens,
            max_output: model.output_tokens,
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(provider.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(provider.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(provider.private).unwrap_or(false),
            active: model.active.or(provider.active).unwrap_or(true),
        }
        .validate()?;
    }
    Ok(())
}
