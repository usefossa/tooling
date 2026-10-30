use super::{
    accounts::{Config, Pool, PoolMode, PoolStatus},
    *,
};
use crate::{Error, Result, ensure, token::TransferFee};

pub fn mul_div(a: u64, b: u64, denominator: u64, ceil: bool) -> Result<u64> {
    ensure(denominator != 0, "nonzero denominator")?;
    let product = u128::from(a) * u128::from(b);
    let denominator = u128::from(denominator);
    (product / denominator + u128::from(ceil && product % denominator != 0))
        .try_into()
        .map_err(|_| Error::Arithmetic)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Arithmetic)
}
fn sub(a: u64, b: u64) -> Result<u64> {
    a.checked_sub(b).ok_or(Error::Arithmetic)
}
fn fee(amount: u64, bps: u16) -> Result<u64> {
    TransferFee {
        basis_points: bps,
        maximum_fee: u64::MAX,
    }
    .fee(amount)
}
pub fn constant_product_output(reserve_in: u64, reserve_out: u64, input: u64) -> Result<u64> {
    ensure(
        reserve_in > 0 && reserve_out > 0 && input > 0,
        "swap reserves or input",
    )?;
    mul_div(reserve_out, input, add(reserve_in, input)?, false)
}
pub fn constant_product_input(reserve_in: u64, reserve_out: u64, output: u64) -> Result<u64> {
    ensure(
        reserve_in > 0 && output > 0 && output < reserve_out,
        "swap reserves or output",
    )?;
    mul_div(reserve_in, output, sub(reserve_out, output)?, true)
}
pub fn integer_sqrt(value: u128) -> u64 {
    if value < 2 {
        return value as u64;
    }
    let mut x = 1u128 << (128 - value.leading_zeros()).div_ceil(2);
    loop {
        let next = (x + value / x) / 2;
        if next >= x {
            return x as u64;
        }
        x = next;
    }
}
fn curve_reserves(pool: &Pool, a_to_b: bool) -> Result<(u64, u64)> {
    let a = add(pool.reserve_a, pool.virtual_a)?;
    let b = add(pool.reserve_b, pool.virtual_b)?;
    Ok(if a_to_b { (a, b) } else { (b, a) })
}
fn trade_gross(effective: u64, creator_bps: u16) -> Result<u64> {
    let mut received = TransferFee {
        basis_points: 125 + creator_bps,
        maximum_fee: u64::MAX,
    }
    .gross(effective)?;
    for _ in 0..4 {
        if sub(
            received,
            add(fee(received, 125)?, fee(received, creator_bps)?)?,
        )? >= effective
        {
            return Ok(received);
        }
        received = add(received, 1)?;
    }
    Err(Error::Arithmetic)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SwapQuote {
    pub gross_input: u64,
    pub received_input: u64,
    pub effective_input: u64,
    pub input_transfer_tax: u64,
    pub protocol_fee: u64,
    pub creator_fee: u64,
    pub gross_output: u64,
    pub net_output: u64,
    pub output_transfer_tax: u64,
    pub unused_input: u64,
    pub reserve_a_after: u64,
    pub reserve_b_after: u64,
}
pub fn swap_exact_input(
    pool: &Pool,
    transfer_fees: [TransferFee; 2],
    args: SwapArgs,
) -> Result<SwapQuote> {
    swap(pool, transfer_fees, args, false)
}
pub fn swap_exact_output(
    pool: &Pool,
    transfer_fees: [TransferFee; 2],
    args: SwapArgs,
) -> Result<SwapQuote> {
    swap(pool, transfer_fees, args, true)
}
fn swap(
    pool: &Pool,
    transfer_fees: [TransferFee; 2],
    args: SwapArgs,
    exact_out: bool,
) -> Result<SwapQuote> {
    ensure(
        args.amount > 0 && (pool.status == PoolStatus::Amm || pool.reserve_b < pool.target_b),
        "swap unavailable",
    )?;
    let curve = pool.status == PoolStatus::Curve;
    let creator_bps = if curve && pool.mode == PoolMode::Standard {
        50
    } else {
        0
    };
    let (input_fee, output_fee, real_out) = if args.a_to_b {
        (transfer_fees[0], transfer_fees[1], pool.reserve_b)
    } else {
        (transfer_fees[1], transfer_fees[0], pool.reserve_a)
    };
    let (reserve_in, reserve_out) = curve_reserves(pool, args.a_to_b)?;
    let wanted_output = if exact_out {
        output_fee.gross(args.amount)?
    } else {
        0
    };
    let mut gross_input = if exact_out {
        let effective = constant_product_input(reserve_in, reserve_out, wanted_output)?;
        let input = input_fee.gross(trade_gross(effective, creator_bps)?)?;
        ensure(input <= args.limit, "maximum swap input")?;
        input
    } else {
        args.amount
    };
    let mut received = input_fee.net(gross_input)?;
    let mut protocol_fee = fee(received, 125)?;
    let mut creator_fee = fee(received, creator_bps)?;
    let mut effective = sub(received, add(protocol_fee, creator_fee)?)?;
    if !exact_out && curve && !args.a_to_b {
        let remaining = sub(pool.target_b, pool.reserve_b)?;
        if effective > remaining {
            received = trade_gross(remaining, creator_bps)?;
            gross_input = input_fee.gross(received)?;
            protocol_fee = fee(received, 125)?;
            creator_fee = fee(received, creator_bps)?;
            effective = sub(received, add(protocol_fee, creator_fee)?)?;
        }
    }
    let maximum_output = constant_product_output(reserve_in, reserve_out, effective)?;
    let gross_output = if exact_out {
        ensure(maximum_output >= wanted_output, "exact output funding")?;
        wanted_output
    } else {
        maximum_output
    };
    let terminal_dust = !exact_out
        && curve
        && !args.a_to_b
        && args.limit == 0
        && add(pool.reserve_b, effective)? == pool.target_b;
    ensure(
        (gross_output > 0 || terminal_dust) && gross_output < real_out,
        "real output liquidity",
    )?;
    if curve && !args.a_to_b {
        let launch_supply = if pool.mode == PoolMode::Model {
            MODEL_SUPPLY_ATOMS
        } else {
            pool.initial_base
        };
        ensure(
            gross_output <= mul_div(launch_supply, 3, 4, false)?,
            "75 percent per-buy supply limit",
        )?;
        ensure(
            sub(pool.reserve_a, gross_output)? >= pool.minimum_a
                && add(pool.reserve_b, effective)? <= pool.target_b,
            "curve graduation capacity",
        )?;
    }
    let net_output = output_fee.net(gross_output)?;
    ensure(
        net_output >= if exact_out { args.amount } else { args.limit },
        "minimum swap output",
    )?;
    let (reserve_a_after, reserve_b_after) = if args.a_to_b {
        (
            add(pool.reserve_a, effective)?,
            sub(pool.reserve_b, gross_output)?,
        )
    } else {
        (
            sub(pool.reserve_a, gross_output)?,
            add(pool.reserve_b, effective)?,
        )
    };
    if curve {
        validate_terminal(
            reserve_a_after,
            reserve_b_after,
            pool.virtual_a,
            pool.virtual_b,
            pool.target_b,
            pool.minimum_a,
        )?;
    }
    add(
        if args.a_to_b {
            pool.protocol_a
        } else {
            pool.protocol_b
        },
        protocol_fee,
    )?;
    add(
        if args.a_to_b {
            pool.creator_a
        } else {
            pool.creator_b
        },
        creator_fee,
    )?;
    Ok(SwapQuote {
        gross_input,
        received_input: received,
        effective_input: effective,
        input_transfer_tax: sub(gross_input, received)?,
        protocol_fee,
        creator_fee,
        gross_output,
        net_output,
        output_transfer_tax: sub(gross_output, net_output)?,
        unused_input: if exact_out {
            0
        } else {
            sub(args.amount, gross_input)?
        },
        reserve_a_after,
        reserve_b_after,
    })
}
fn validate_terminal(
    a: u64,
    b: u64,
    virtual_a: u64,
    virtual_b: u64,
    target: u64,
    minimum: u64,
) -> Result<()> {
    let budget = sub(target, b)?;
    let output = if budget == 0 {
        0
    } else {
        constant_product_output(add(b, virtual_b)?, add(a, virtual_a)?, budget)?
    };
    let remaining = sub(a, output)?;
    ensure(remaining >= minimum, "terminal minimum reserve")?;
    let matched = mul_div(
        target,
        add(remaining, virtual_a)?,
        add(target, virtual_b)?,
        true,
    )?;
    ensure(
        matched <= remaining && u128::from(matched) * u128::from(target) >= 1_001u128.pow(2),
        "solvent graduation",
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CurvePreset {
    pub initial_base: u64,
    pub virtual_a: u64,
    pub virtual_b: u64,
    pub target_b: u64,
    pub minimum_a: u64,
}
pub fn model_preset() -> CurvePreset {
    CurvePreset {
        initial_base: MODEL_CURVE_SUPPLY_ATOMS,
        virtual_a: MODEL_CURVE_SUPPLY_ATOMS,
        virtual_b: MODEL_VIRTUAL_USDC_ATOMS,
        target_b: MODEL_RAISE_USDC_ATOMS,
        minimum_a: (u128::from(MODEL_CURVE_SUPPLY_ATOMS)
            * u128::from(MODEL_VIRTUAL_USDC_ATOMS - MODEL_RAISE_USDC_ATOMS)
            / u128::from(MODEL_VIRTUAL_USDC_ATOMS + MODEL_RAISE_USDC_ATOMS))
            as u64,
    }
}
pub fn public_preset(config: &Config, now: u64) -> Result<CurvePreset> {
    public_preset_for_target(public_target(config, now)?)
}
pub fn public_preset_for_target(target_b: u64) -> Result<CurvePreset> {
    ensure(target_b > 0, "public launch target")?;
    let denominator = PUBLIC_CURVE_SALE_ATOMS - PUBLIC_LIQUIDITY_ATOMS;
    let preset = CurvePreset {
        initial_base: PUBLIC_SUPPLY_ATOMS,
        virtual_a: mul_div(
            PUBLIC_LIQUIDITY_ATOMS,
            PUBLIC_LIQUIDITY_ATOMS,
            denominator,
            false,
        )?,
        virtual_b: mul_div(target_b, PUBLIC_LIQUIDITY_ATOMS, denominator, true)?,
        target_b,
        minimum_a: PUBLIC_LIQUIDITY_ATOMS,
    };
    let sold = constant_product_output(
        preset.virtual_b,
        add(preset.initial_base, preset.virtual_a)?,
        target_b,
    )?;
    ensure(
        sub(PUBLIC_CURVE_SALE_ATOMS, sold)? <= PUBLIC_SUPPLY_ATOMS / 10_000,
        "quote mint precision",
    )?;
    ensure(
        target_b
            <= constant_product_input(
                preset.virtual_b,
                add(preset.initial_base, preset.virtual_a)?,
                sub(preset.initial_base, preset.minimum_a)?,
            )?,
        "launch target capacity",
    )?;
    validate_terminal(
        preset.initial_base,
        0,
        preset.virtual_a,
        preset.virtual_b,
        target_b,
        preset.minimum_a,
    )?;
    Ok(preset)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GraduationQuote {
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub burn_base_atoms: u64,
    pub locked_lp: u64,
}
pub fn graduation(pool: &Pool) -> Result<GraduationQuote> {
    ensure(
        pool.status == PoolStatus::Curve
            && pool.reserve_b >= pool.target_b
            && pool.reserve_a >= pool.minimum_a,
        "graduation readiness",
    )?;
    let matched = mul_div(
        pool.reserve_b,
        add(pool.reserve_a, pool.virtual_a)?,
        add(pool.reserve_b, pool.virtual_b)?,
        true,
    )?;
    ensure(
        matched > 0 && matched <= pool.reserve_a,
        "graduation base liquidity",
    )?;
    let total = integer_sqrt(u128::from(matched) * u128::from(pool.reserve_b));
    ensure(total > 1000, "graduation minimum liquidity")?;
    Ok(GraduationQuote {
        reserve_a: matched,
        reserve_b: pool.reserve_b,
        burn_base_atoms: sub(pool.reserve_a, matched)?,
        locked_lp: total,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LiquidityQuote {
    pub lp_amount: u64,
    pub gross_a: u64,
    pub gross_b: u64,
    pub net_a: u64,
    pub net_b: u64,
}
pub fn initial_liquidity(
    gross_a: u64,
    gross_b: u64,
    fees: [TransferFee; 2],
) -> Result<LiquidityQuote> {
    let net_a = fees[0].net(gross_a)?;
    let net_b = fees[1].net(gross_b)?;
    ensure(net_a > 0 && net_b > 0, "initial liquidity deposits")?;
    Ok(LiquidityQuote {
        lp_amount: integer_sqrt(u128::from(net_a) * u128::from(net_b)),
        gross_a,
        gross_b,
        net_a,
        net_b,
    })
}
pub fn add_liquidity(
    pool: &Pool,
    fees: [TransferFee; 2],
    lp_supply: u64,
    args: LiquidityArgs,
) -> Result<LiquidityQuote> {
    ensure(
        pool.status == PoolStatus::Amm && args.amount > 0,
        "AMM liquidity",
    )?;
    let total = add(lp_supply, pool.locked_lp)?;
    let quote = if total == 0 {
        ensure(
            pool.reserve_a == 0 && pool.reserve_b == 0,
            "empty direct pool",
        )?;
        let quote = initial_liquidity(args.bound_a, args.bound_b, fees)?;
        ensure(quote.lp_amount == args.amount, "exact initial LP amount")?;
        quote
    } else {
        let net_a = mul_div(pool.reserve_a, args.amount, total, true)?;
        let net_b = mul_div(pool.reserve_b, args.amount, total, true)?;
        LiquidityQuote {
            lp_amount: args.amount,
            gross_a: fees[0].gross(net_a)?,
            gross_b: fees[1].gross(net_b)?,
            net_a,
            net_b,
        }
    };
    ensure(
        quote.net_a > 0
            && quote.net_b > 0
            && quote.gross_a <= args.bound_a
            && quote.gross_b <= args.bound_b,
        "maximum liquidity deposits",
    )?;
    add(pool.reserve_a, quote.net_a)?;
    add(pool.reserve_b, quote.net_b)?;
    add(lp_supply, args.amount)?;
    Ok(quote)
}
pub fn remove_liquidity(
    pool: &Pool,
    fees: [TransferFee; 2],
    lp_supply: u64,
    args: LiquidityArgs,
) -> Result<LiquidityQuote> {
    ensure(
        pool.status == PoolStatus::Amm && args.amount > 0 && args.amount <= lp_supply,
        "redeemable LP supply",
    )?;
    let total = add(lp_supply, pool.locked_lp)?;
    let gross_a = mul_div(pool.reserve_a, args.amount, total, false)?;
    let gross_b = mul_div(pool.reserve_b, args.amount, total, false)?;
    let quote = LiquidityQuote {
        lp_amount: args.amount,
        gross_a,
        gross_b,
        net_a: fees[0].net(gross_a)?,
        net_b: fees[1].net(gross_b)?,
    };
    ensure(
        gross_a > 0 && gross_b > 0 && quote.net_a >= args.bound_a && quote.net_b >= args.bound_b,
        "minimum liquidity withdrawals",
    )?;
    Ok(quote)
}
pub fn collect_fees(pool: &Pool, fees: [TransferFee; 2], protocol: bool) -> Result<[u64; 2]> {
    let (a, b) = if protocol {
        (pool.protocol_a, pool.protocol_b)
    } else {
        (pool.creator_a, pool.creator_b)
    };
    ensure(a > 0 || b > 0, "accrued fees")?;
    Ok([fees[0].net(a)?, fees[1].net(b)?])
}
pub fn cumulative_price(pool: &Pool, now: u64) -> Result<u128> {
    ensure(
        pool.mode == PoolMode::Model && pool.status == PoolStatus::Amm && pool.locked_lp > 0,
        "graduated model reference pool",
    )?;
    let elapsed = sub(now, pool.price_timestamp)?;
    ensure(elapsed <= u64::from(u32::MAX), "oracle elapsed time")?;
    let model_reserve = add(pool.reserve_a, pool.virtual_a)?;
    let usdc_reserve = add(pool.reserve_b, pool.virtual_b)?;
    ensure(model_reserve > 0 && usdc_reserve > 0, "oracle reserves")?;
    let numerator = u128::from(model_reserve) << 32;
    let denominator = u128::from(usdc_reserve);
    let model_per_usdc_x32 = numerator / denominator + u128::from(numerator % denominator != 0);
    Ok(pool
        .price_cumulative
        .wrapping_add(model_per_usdc_x32 * u128::from(elapsed)))
}
pub fn refresh_quote(config: &Config, reference: &Pool, now: u64) -> Result<Config> {
    ensure(
        config.enabled && config.quote_mint == reference.mint_a,
        "enabled quote reference",
    )?;
    let mut next = config.clone();
    let elapsed = sub(now, config.oracle_timestamp)?;
    if config.oracle_timestamp == 0 || elapsed > ORACLE_WINDOW_SECONDS * 2 {
        next.oracle_cumulative = cumulative_price(reference, now)?;
        next.oracle_timestamp = now;
        next.quote_per_usdc_x32 = 0;
        next.quote_price_timestamp = 0;
    } else {
        ensure(
            elapsed >= ORACLE_WINDOW_SECONDS,
            "oracle observation window",
        )?;
        let cumulative = cumulative_price(reference, now)?;
        let observed = cumulative.wrapping_sub(config.oracle_cumulative);
        let elapsed = u128::from(elapsed);
        let average = observed / elapsed + u128::from(observed % elapsed != 0);
        ensure(average > 0, "positive oracle average")?;
        next.oracle_cumulative = cumulative;
        next.oracle_timestamp = now;
        next.quote_per_usdc_x32 = average;
        next.quote_price_timestamp = now;
    }
    Ok(next)
}
pub fn public_target(config: &Config, now: u64) -> Result<u64> {
    ensure(
        config.enabled
            && config.reference_pool != Pubkey::default()
            && config.quote_per_usdc_x32 > 0
            && sub(now, config.quote_price_timestamp)? <= ORACLE_WINDOW_SECONDS,
        "fresh approved quote price",
    )?;
    let numerator = u128::from(PUBLIC_RAISE_USDC_ATOMS)
        .checked_mul(config.quote_per_usdc_x32)
        .ok_or(Error::Arithmetic)?;
    let scale = 1u128 << 32;
    (numerator / scale + u128::from(numerator % scale != 0))
        .try_into()
        .map_err(|_| Error::Arithmetic)
}
pub fn action_fee(payer: Pubkey, treasury: Pubkey) -> u64 {
    if payer == treasury {
        0
    } else {
        ACTION_FEE_LAMPORTS
    }
}
