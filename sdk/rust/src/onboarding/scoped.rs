use super::*;
use crate::{
    accounts::{CapacityBook, ProviderList, key, n32},
    scoped::{ListRouting, ListTerms, ProviderModelCapacity, ProviderPolicy},
};
use sha2::{Digest, Sha256};

pub async fn plan_for_list(
    rpc: &RpcClient,
    p: &Protocol,
    c: &OnboardConfig,
    list: Pubkey,
) -> Result<OnboardingPlan> {
    let market = rpc.validate_market(p).await?;
    ensure(
        market.list_routing_enabled,
        "list routing activation required",
    )?;
    c.document.encode()?;
    let mut seen = std::collections::HashSet::new();
    for model in &c.models {
        ensure(
            !model.id.is_empty() && seen.insert(&model.id),
            "unique model ids",
        )?;
        model.document.encode()?;
        model.terms.validate()?;
    }
    let mut plan = OnboardingPlan {
        steps: vec![],
        blockers: vec![],
    };
    let profile = rpc
        .account::<Provider>(
            address::provider(&p.program, &p.market, &c.provider).0,
            p.program,
        )
        .await?;
    if let Some(profile) = profile {
        if profile.document != c.document {
            plan.steps.push(PlanStep::new(
                "update provider metadata",
                vec![p.update_provider_metadata(c.provider, &c.document)?],
            ));
        }
    } else {
        plan.steps.push(PlanStep::new(
            "register provider",
            vec![p.initialize_provider(c.provider, c.payer, &c.document)?],
        ));
    }
    let policy = rpc
        .account::<ProviderPolicy>(
            address::provider_policy(&p.program, &p.market, &c.provider).0,
            p.program,
        )
        .await?;
    if policy.is_none() {
        plan.steps.push(PlanStep::new(
            "initialize provider policy",
            vec![p.initialize_provider_policy(c.provider, c.payer, BookingLimits::default())?],
        ));
    }
    if policy.as_ref().is_some_and(|policy| policy.updating) {
        plan.blockers
            .push("Complete provider policy update before enabling list routing".into());
    }
    if market.provider_limits_enabled {
        use crate::provider_limits::{
            ProviderLimitIndex, ProviderLimitPage, ProviderLimits, RequestLimits,
        };
        let identity = rpc
            .account::<ProviderLimitIndex>(
                address::provider_limits(&p.program, &p.market, &c.provider).0,
                p.program,
            )
            .await?;
        if identity.is_none() {
            let page = Sha256::digest(c.provider.as_ref())[0] & 31;
            let state = rpc
                .account::<ProviderLimitPage>(
                    address::provider_limit_page(&p.program, &p.market, page).0,
                    p.program,
                )
                .await?;
            let mut ix = Vec::new();
            if let Some(state) = state {
                if state.entries.len() == state.capacity as usize {
                    ix.push(p.grow_provider_limit_page(c.payer, page, state.capacity + 64)?);
                }
            } else {
                ix.push(p.initialize_provider_limit_page(c.payer, page, 64)?);
            }
            let wallet = policy
                .as_ref()
                .map(|policy| RequestLimits {
                    concurrency: policy.limits.wallet_booking_capacity,
                    quota: policy.limits.wallet_bookings_per_window,
                    quota_window_seconds: policy.limits.wallet_window_seconds,
                })
                .unwrap_or_default();
            ix.push(p.initialize_provider_limits(
                c.provider,
                c.payer,
                page,
                ProviderLimits {
                    global: RequestLimits::default(),
                    wallet,
                },
            )?);
            plan.steps.push(PlanStep::new(
                "initialize provider-wide concurrency and quota",
                ix,
            ));
        }
    }
    let membership = rpc.require_account::<ProviderList>(list, p.program).await?;
    ensure(membership.market == p.market, "provider list market")?;
    let owns_list = membership.authority == c.provider;
    let routing = rpc
        .account::<ListRouting>(address::list_routing(&p.program, &list).0, p.program)
        .await?;
    if routing.is_none() {
        if owns_list {
            plan.steps.push(PlanStep::new(
                "initialize list routing",
                vec![p.initialize_list_routing(
                    list,
                    c.provider,
                    c.payer,
                    membership.limit.min(16),
                )?],
            ));
        } else {
            plan.blockers
                .push(format!("List owner must initialize routing for {list}"));
        }
    }
    if !membership.contains(&c.provider) {
        if owns_list {
            plan.steps.push(PlanStep::new(
                "admit provider to selected list",
                vec![p.update_routing_list(list, c.provider, c.provider, true, true)],
            ));
        } else {
            plan.blockers
                .push(format!("List owner must admit provider {}", c.provider));
        }
    }
    let enabled = routing.as_ref().is_some_and(|routing| {
        routing
            .entries
            .iter()
            .any(|entry| entry.provider == c.provider && entry.active)
    });
    if !enabled
        && (routing.is_some() || owns_list)
        && (membership.contains(&c.provider) || owns_list)
    {
        let mut ix = vec![];
        if let Some(routing) = &routing
            && routing.entries.len() == routing.capacity as usize
            && !routing
                .entries
                .iter()
                .any(|entry| entry.provider == c.provider)
        {
            if owns_list {
                ix.push(p.grow_list_routing(list, c.provider, c.payer, routing.capacity + 1));
            } else {
                plan.blockers
                    .push("List owner must grow routing registry".into());
            }
        }
        ix.push(p.enable_policy_mirror(list, c.provider));
        plan.steps
            .push(PlanStep::new("enable selected-list policy", ix));
    }
    for desired in &c.models {
        let id = address::model_id(&desired.id);
        let at = address::model(&p.program, &p.market, &id).0;
        let existing = rpc.account::<Model>(at, p.program).await?;
        let (model, legacy) = if let Some(model) = existing {
            ensure(model.market == p.market && model.id == id, "model binding")?;
            let metadata = rpc
                .require_account::<ModelMetadata>(
                    address::model_metadata(&p.program, &at).0,
                    p.program,
                )
                .await?;
            if metadata.document != desired.document {
                if metadata.authority == c.provider {
                    plan.steps.push(PlanStep::new(
                        format!("update model metadata {}", desired.id),
                        vec![p.update_model_metadata(at, c.provider, &desired.document)?],
                    ));
                } else {
                    plan.blockers.push(format!(
                        "Model {} metadata belongs to {}",
                        desired.id, metadata.authority
                    ));
                }
            }
            let keys = ModelKeys::from_account(at, &model);
            let legacy = if market.provider_limits_enabled {
                0
            } else {
                legacy_active(rpc, p, keys, c.provider).await?
            };
            (keys, legacy)
        } else {
            let terms = Keypair::new();
            let capacity = Keypair::new();
            let model = ModelKeys {
                model: at,
                terms: terms.pubkey(),
                capacity: capacity.pubkey(),
                group: Pubkey::default(),
            };
            plan.steps.push(PlanStep {
                label: format!("initialize model {}", desired.id),
                instructions: vec![p.initialize_model(InitializeModel {
                    model,
                    provider: c.provider,
                    payer: c.payer,
                    id,
                    limit: c.book_limit,
                    document: &desired.document,
                })?],
                signers: vec![terms, capacity],
            });
            (model, 0)
        };
        let book = rpc
            .account::<ListTerms>(address::list_terms(&p.program, &list, &at).0, p.program)
            .await?;
        if book.is_none() {
            if owns_list {
                plan.steps.push(PlanStep::new(
                    format!("initialize list books {}", desired.id),
                    vec![p.initialize_list_books(
                        list,
                        at,
                        c.provider,
                        c.payer,
                        c.book_limit.min(64),
                    )?],
                ));
            } else {
                plan.blockers.push(format!(
                    "List owner must initialize list books for {}",
                    desired.id
                ));
                continue;
            }
        }
        let old = book.as_ref().and_then(|book| {
            book.entries
                .iter()
                .find(|entry| entry.offer.provider == c.provider)
        });
        if !market.provider_limits_enabled {
            let budget = rpc
                .account::<ProviderModelCapacity>(
                    address::provider_capacity(&p.program, &at, &c.provider).0,
                    p.program,
                )
                .await?;
            let old_allocation = old.map(|entry| entry.offer.terms.capacity).unwrap_or(0);
            let allocated = budget
                .as_ref()
                .map(|budget| budget.allocated)
                .unwrap_or(0)
                .checked_sub(old_allocation)
                .and_then(|value| value.checked_add(desired.terms.capacity))
                .ok_or(Error::Arithmetic)?;
            let total = budget
                .as_ref()
                .map(|budget| budget.total)
                .unwrap_or(desired.terms.capacity);
            if u64::from(allocated) + u64::from(legacy) > u64::from(total) {
                plan.blockers.push(format!("Insufficient global capacity for {}: reduce list allocation, drain legacy bookings, or explicitly configure total capacity",desired.id));
                continue;
            }
            if budget.is_none() {
                plan.steps.push(PlanStep::new(
                    format!("initialize capacity {}", desired.id),
                    vec![p.initialize_model_capacity(model, c.provider, c.payer, total)],
                ));
            } else if budget
                .as_ref()
                .is_some_and(|budget| budget.legacy_active != legacy)
            {
                plan.steps.push(PlanStep::new(
                    format!("sync drained capacity {}", desired.id),
                    vec![p.sync_legacy_capacity(model, c.provider)],
                ));
            }
        }
        if old.is_none_or(|old| old.offer.terms != desired.terms)
            || market.provider_limits_enabled
                && book.as_ref().is_some_and(|b| b.limits_version == 0)
        {
            let mut ix = vec![];
            if let Some(book) = &book
                && old.is_none()
                && book.entries.len() == book.limit as usize
            {
                if owns_list {
                    ix.push(p.grow_list_books(list, at, c.provider, c.payer, book.limit + 1));
                } else {
                    plan.blockers.push(format!(
                        "List owner must grow model books for {}",
                        desired.id
                    ));
                }
            }
            let payout = old
                .map(|old| old.offer.payout)
                .unwrap_or_else(|| address::associated_token(&p.mint, &c.provider));
            if old.is_none() {
                ix.push(create_associated_token(c.payer, c.provider, p.mint));
            }
            ix.push(if market.provider_limits_enabled {
                p.set_provider_list_offer(list, at, c.provider, payout, desired.terms)?
            } else {
                p.set_list_offer(list, at, c.provider, payout, desired.terms)?
            });
            plan.steps
                .push(PlanStep::new(format!("set list offer {}", desired.id), ix));
        }
    }
    Ok(plan)
}

async fn legacy_active(
    rpc: &RpcClient,
    p: &Protocol,
    m: ModelKeys,
    provider: Pubkey,
) -> Result<u32> {
    let result = rpc.multiple_accounts(&[m.terms, m.capacity]).await?;
    let raw = result.value[0]
        .as_ref()
        .ok_or(Error::MissingAccount(m.terms))?;
    let capacity = CapacityBook::decode(
        &m.capacity,
        result.value[1]
            .as_ref()
            .ok_or(Error::MissingAccount(m.capacity))?,
        &p.program,
    )?;
    let d = &raw.data;
    ensure(
        raw.owner == p.program && !raw.executable && d.len() >= 48 && key(d, 8) == m.model,
        "legacy book binding",
    )?;
    let stride = if d.starts_with(&Sha256::digest(b"account:FilteredTerms")[..8]) {
        132
    } else {
        ensure(
            d.starts_with(crate::accounts::TermsBook::MAGIC),
            "legacy terms layout",
        )?;
        136
    };
    let count = n32(d, 44) as usize;
    let limit = n32(d, 40);
    ensure(
        count == capacity.entries.len()
            && limit == capacity.limit
            && d.len() == 48 + limit as usize * stride,
        "legacy book count",
    )?;
    Ok(d[48..48 + count * stride]
        .chunks_exact(stride)
        .position(|row| key(row, 0) == provider)
        .map(|i| capacity.entries[i].active)
        .unwrap_or(0))
}
