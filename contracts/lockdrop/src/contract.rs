use std::convert::TryInto;

use astroport::asset::{addr_validate_to_lower, Asset, AssetInfo};
use astroport::generator::{
    ExecuteMsg as GenExecuteMsg, PendingTokenResponse, QueryMsg as GenQueryMsg, RewardInfoResponse,
};
use cosmwasm_std::{
    attr, entry_point, from_binary, to_binary, Addr, Binary, Coin, CosmosMsg, Decimal, Decimal256,
    Deps, DepsMut, Env, MessageInfo, Order, Response, StdError, StdResult, SubMsg, Uint128,
    Uint256, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg};
use cw_storage_plus::{Path, U64Key};

use crate::migration::ASSET_POOLS_V101;
use astroport_periphery::auction::Cw20HookMsg::DelegateAstroTokens;
use astroport_periphery::lockdrop::{
    CallbackMsg, ConfigResponse, Cw20HookMsg, ExecuteMsg, InstantiateMsg, LockUpInfoResponse,
    LockUpInfoSummary, MigrateMsg, MigrationInfo, PoolResponse, QueryMsg, StateResponse,
    UpdateConfigMsg, UserInfoResponse, UserInfoWithListResponse,
};

use crate::state::{
    Config, LockupInfo, PoolInfo, State, ASSET_POOLS, CONFIG, LOCKUP_INFO, STATE,
    TOTAL_ASSET_REWARD_INDEX, USERS_ASSET_REWARD_INDEX, USER_INFO,
};

const SECONDS_PER_WEEK: u64 = 86400 * 7;

// version info for migration info
const CONTRACT_NAME: &str = "astroport_lockdrop";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

//----------------------------------------------------------------------------------------
// Entry Points
//----------------------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    // CHECK :: init_timestamp needs to be valid
    if env.block.time.seconds() > msg.init_timestamp {
        return Err(StdError::generic_err(format!(
            "Invalid init_timestamp. Current timestamp : {}",
            env.block.time.seconds()
        )));
    }

    // CHECK :: min_lock_duration , max_lock_duration need to be valid (min_lock_duration < max_lock_duration)
    if msg.max_lock_duration < msg.min_lock_duration || msg.min_lock_duration == 0u64 {
        return Err(StdError::generic_err("Invalid Lockup durations"));
    }

    // CHECK ::Weekly divider/multiplier cannot be 0
    if msg.weekly_divider == 0u64 || msg.weekly_multiplier == 0u64 {
        return Err(StdError::generic_err(
            "weekly divider/multiplier cannot be 0",
        ));
    }

    let config = Config {
        owner: msg
            .owner
            .map(|v| deps.api.addr_validate(&v))
            .transpose()?
            .unwrap_or(info.sender),
        astro_token: None,
        auction_contract: None,
        generator: None,
        init_timestamp: msg.init_timestamp,
        deposit_window: msg.deposit_window,
        withdrawal_window: msg.withdrawal_window,
        min_lock_duration: msg.min_lock_duration,
        max_lock_duration: msg.max_lock_duration,
        weekly_multiplier: msg.weekly_multiplier,
        weekly_divider: msg.weekly_divider,
        lockdrop_incentives: Uint128::zero(),
        max_positions_per_user: msg.max_positions_per_user,
    };

    let state = State {
        total_incentives_share: 0,
        total_astro_delegated: Uint128::zero(),
        are_claims_allowed: false,
    };

    CONFIG.save(deps.storage, &config)?;
    STATE.save(deps.storage, &state)?;
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),

        ExecuteMsg::UpdateConfig { new_config } => handle_update_config(deps, info, new_config),
        ExecuteMsg::InitializePool {
            terraswap_lp_token,
            incentives_share,
        } => handle_initialize_pool(deps, env, info, terraswap_lp_token, incentives_share),
        ExecuteMsg::UpdatePool {
            terraswap_lp_token,
            incentives_share,
        } => handle_update_pool(deps, env, info, terraswap_lp_token, incentives_share),

        ExecuteMsg::MigrateLiquidity {
            terraswap_lp_token,
            astroport_pool_addr,
            slippage_tolerance,
        } => handle_migrate_liquidity(
            deps,
            env,
            info,
            terraswap_lp_token,
            astroport_pool_addr,
            slippage_tolerance,
        ),

        ExecuteMsg::StakeLpTokens { terraswap_lp_token } => {
            handle_stake_lp_tokens(deps, env, info, terraswap_lp_token)
        }
        ExecuteMsg::EnableClaims {} => handle_enable_claims(deps, env, info),
        ExecuteMsg::DelegateAstroToAuction { amount } => {
            handle_delegate_astro_to_auction(deps, env, info, amount)
        }
        ExecuteMsg::WithdrawFromLockup {
            terraswap_lp_token,
            duration,
            amount,
        } => handle_withdraw_from_lockup(deps, env, info, terraswap_lp_token, duration, amount),
        ExecuteMsg::ClaimRewardsAndOptionallyUnlock {
            terraswap_lp_token,
            duration,
            withdraw_lp_stake,
        } => handle_claim_rewards_and_unlock_for_lockup(
            deps,
            env,
            info,
            terraswap_lp_token,
            duration,
            withdraw_lp_stake,
        ),

        ExecuteMsg::Callback(msg) => _handle_callback(deps, env, info, msg),
        ExecuteMsg::ClaimAssetReward {
            recipient,
            terraswap_lp_token,
            duration,
        } => {
            let recipient = recipient.map_or_else(
                || Ok(info.sender.clone()),
                |recip_addr| addr_validate_to_lower(deps.api, &recip_addr),
            )?;
            handle_claim_asset_reward(
                deps.as_ref(),
                env,
                info.sender,
                recipient,
                terraswap_lp_token,
                duration,
            )
        }
        ExecuteMsg::TogglePoolRewards {
            terraswap_lp_token,
            enable,
        } => handle_toggle_rewards(deps, info, terraswap_lp_token, enable),
    }
}

pub fn receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, StdError> {
    let user_address = deps.api.addr_validate(&cw20_msg.sender)?;

    // CHECK :: Tokens sent > 0
    if cw20_msg.amount == Uint128::zero() {
        return Err(StdError::generic_err(
            "Number of tokens sent should be > 0 ",
        ));
    }

    let amount = cw20_msg.amount;

    match from_binary(&cw20_msg.msg)? {
        Cw20HookMsg::IncreaseLockup { duration } => {
            handle_increase_lockup(deps, env, info, user_address, duration, amount)
        }
        Cw20HookMsg::IncreaseAstroIncentives {} => {
            handle_increasing_astro_incentives(deps, env, info, amount)
        }
    }
}

fn _handle_callback(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: CallbackMsg,
) -> StdResult<Response> {
    // Only the contract itself can call callbacks
    if info.sender != env.contract.address {
        return Err(StdError::generic_err(
            "callbacks cannot be invoked externally",
        ));
    }
    match msg {
        CallbackMsg::UpdatePoolOnDualRewardsClaim {
            terraswap_lp_token,
            prev_astro_balance,
            prev_proxy_reward_balance,
        } => update_pool_on_dual_rewards_claim(
            deps,
            env,
            terraswap_lp_token,
            prev_astro_balance,
            prev_proxy_reward_balance,
        ),
        CallbackMsg::WithdrawUserLockupRewardsCallback {
            terraswap_lp_token,
            user_address,
            duration,
            withdraw_lp_stake,
        } => callback_withdraw_user_rewards_for_lockup_optional_withdraw(
            deps,
            env,
            terraswap_lp_token,
            user_address,
            duration,
            withdraw_lp_stake,
        ),
        CallbackMsg::WithdrawLiquidityFromTerraswapCallback {
            terraswap_lp_token,
            astroport_pool,
            prev_assets,
            slippage_tolerance,
        } => callback_deposit_liquidity_in_astroport(
            deps,
            env,
            terraswap_lp_token,
            astroport_pool,
            prev_assets,
            slippage_tolerance,
        ),
        CallbackMsg::DistributeAssetReward {
            previous_balance,
            terraswap_lp_token,
            user_address,
            recipient,
            lock_duration,
        } => callback_distribute_asset_reward(
            deps,
            env,
            previous_balance,
            terraswap_lp_token,
            recipient,
            user_address,
            lock_duration,
        ),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::State {} => to_binary(&query_state(deps)?),
        QueryMsg::Pool { terraswap_lp_token } => to_binary(&query_pool(deps, terraswap_lp_token)?),
        QueryMsg::UserInfo { address } => to_binary(&query_user_info(deps, env, address)?),
        QueryMsg::UserInfoWithLockupsList { address } => {
            to_binary(&query_user_info_with_lockups_list(deps, env, address)?)
        }
        QueryMsg::LockUpInfo {
            user_address,
            terraswap_lp_token,
            duration,
        } => to_binary(&query_lockup_info(
            deps,
            &env,
            &user_address,
            terraswap_lp_token,
            duration,
        )?),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    use std::str;

    let contract_version = get_contract_version(deps.storage)?;

    match contract_version.contract.as_ref() {
        "astroport_lockdrop" => match contract_version.version.as_ref() {
            "1.0.1" => {
                let pools = ASSET_POOLS_V101
                    .range(deps.storage, None, None, Order::Ascending)
                    .map(|pair_result| {
                        pair_result.map(|(addr_serialized, pool_info)| {
                            let addr_str = str::from_utf8(&addr_serialized)
                                .map_err(|_| StdError::generic_err("Deserialization error"))?;
                            let addr = addr_validate_to_lower(deps.as_ref().api, addr_str)?;
                            Ok((addr, pool_info))
                        })?
                    })
                    .collect::<StdResult<Vec<_>>>()?;
                for (key, pool) in pools {
                    let new_pool_info = PoolInfo {
                        terraswap_pool: pool.terraswap_pool,
                        terraswap_amount_in_lockups: pool.terraswap_amount_in_lockups,
                        migration_info: pool.migration_info,
                        incentives_share: pool.incentives_share,
                        weighted_amount: pool.weighted_amount,
                        generator_astro_per_share: pool.generator_astro_per_share,
                        generator_proxy_per_share: pool.generator_proxy_per_share,
                        is_staked: pool.is_staked,
                        has_asset_rewards: false,
                    };
                    ASSET_POOLS.save(deps.storage, &key, &new_pool_info)?
                }
            }
            _ => return Err(StdError::generic_err("Migration error")),
        },
        _ => return Err(StdError::generic_err("Migration error")),
    };

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new().add_attributes(vec![
        ("previous_contract_name", &contract_version.contract),
        ("previous_contract_version", &contract_version.version),
        ("current_contract_name", &CONTRACT_NAME.to_string()),
        ("current_contract_version", &CONTRACT_VERSION.to_string()),
    ]))
}

//----------------------------------------------------------------------------------------
// Handle Functions
//----------------------------------------------------------------------------------------

/// @dev Admin function to update Configuration parameters
/// @param new_config : Same as UpdateConfigMsg struct
pub fn handle_update_config(
    deps: DepsMut,
    info: MessageInfo,
    new_config: UpdateConfigMsg,
) -> StdResult<Response> {
    let mut config = CONFIG.load(deps.storage)?;
    let mut attributes = vec![attr("action", "update_config")];

    // CHECK :: Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    if let Some(owner) = new_config.owner {
        config.owner = deps.api.addr_validate(&owner)?;
        attributes.push(attr("new_owner", owner.as_str()))
    };

    if let Some(astro_addr) = new_config.astro_token_address {
        if config.astro_token.is_some() {
            return Err(StdError::generic_err("ASTRO token already set"));
        }
        config.astro_token = Some(deps.api.addr_validate(&astro_addr)?);
        attributes.push(attr("new_astro_token", astro_addr))
    };

    if let Some(auction) = new_config.auction_contract_address {
        match config.auction_contract {
            Some(_) => {
                return Err(StdError::generic_err("Auction contract already set."));
            }
            None => {
                config.auction_contract = Some(deps.api.addr_validate(&auction)?);
                attributes.push(attr("auction_contract", auction))
            }
        }
    };

    if let Some(generator) = new_config.generator_address {
        // If generator is set, we check is any LP tokens are currently staked before updating generator address
        if config.generator.is_some() {
            for pool in ASSET_POOLS
                .keys(deps.storage, None, None, Order::Ascending)
                .map(|v| {
                    Addr::unchecked(String::from_utf8(v).expect("Addr deserialization error!"))
                })
            {
                let pool_info = ASSET_POOLS.load(deps.storage, &pool)?;
                if pool_info.is_staked {
                    return Err(StdError::generic_err(format!(
                        "{} astro LP tokens already staked. Unstake them before updating generator",
                        pool.to_string()
                    )));
                }
            }
        }

        config.generator = Some(deps.api.addr_validate(&generator)?);
        attributes.push(attr("new_generator", generator))
    }

    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attributes(attributes))
}

/// @dev Facilitates increasing ASTRO incentives that are to be distributed as Lockdrop participation reward
/// @params amount : Number of ASTRO tokens which are to be added to current incentives
pub fn handle_increasing_astro_incentives(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, StdError> {
    let mut config = CONFIG.load(deps.storage)?;

    if &info.sender
        != config
            .astro_token
            .as_ref()
            .ok_or_else(|| StdError::generic_err("Astro token should be set!"))?
    {
        return Err(StdError::generic_err("Only astro tokens are received!"));
    }

    if env.block.time.seconds()
        >= config.init_timestamp + config.deposit_window + config.withdrawal_window
    {
        return Err(StdError::generic_err("ASTRO is already being distributed"));
    };

    // Anyone can increase astro incentives
    config.lockdrop_incentives += amount;

    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "astro_incentives_increased")
        .add_attribute("amount", amount))
}

/// @dev Admin function to initialize new LP Pool
/// @param terraswap_lp_token : terraswap LP token address
/// @param incentives_share : parameter defining share of total ASTRO incentives are allocated for this pool
pub fn handle_initialize_pool(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
    incentives_share: u64,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // CHECK ::: Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    // CHECK :: Is lockdrop deposit window closed
    if env.block.time.seconds() >= config.init_timestamp + config.deposit_window {
        return Err(StdError::generic_err(
            "Pools cannot be added post deposit window closure",
        ));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;

    // CHECK ::: Is LP Token Pool already initialized
    if ASSET_POOLS
        .may_load(deps.storage, &terraswap_lp_token)?
        .is_some()
    {
        return Err(StdError::generic_err("Already supported"));
    }

    let terraswap_pool = {
        let res: Option<cw20::MinterResponse> = deps
            .querier
            .query_wasm_smart(&terraswap_lp_token, &Cw20QueryMsg::Minter {})?;
        deps.api
            .addr_validate(&res.expect("No minter for the LP token!").minter)?
    };

    // POOL INFO :: Initialize new pool
    let pool_info = PoolInfo {
        terraswap_pool,
        terraswap_amount_in_lockups: Default::default(),
        migration_info: None,
        incentives_share,
        weighted_amount: Default::default(),
        generator_astro_per_share: Default::default(),
        generator_proxy_per_share: Default::default(),
        is_staked: false,
        has_asset_rewards: false,
    };
    // STATE UPDATE :: Save state and PoolInfo
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

    state.total_incentives_share += incentives_share;
    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "initialize_pool"),
        attr("terraswap_lp_token", terraswap_lp_token),
        attr("incentives_share", incentives_share.to_string()),
    ]))
}

/// @dev Admin function to update LP Pool Configuration
/// @param terraswap_lp_token : Parameter to identify the pool. Equals pool's terraswap Lp token address
/// @param incentives_share : parameter defining share of total ASTRO incentives are allocated for this pool
pub fn handle_update_pool(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
    incentives_share: u64,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // CHECK ::: Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    // CHECK :: Is lockdrop deposit window closed
    if env.block.time.seconds() >= config.init_timestamp + config.deposit_window {
        return Err(StdError::generic_err(
            "Pools cannot be updated post deposit window closure",
        ));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;

    // CHECK ::: Is LP Token Pool initialized
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    // CHECK ::: Incentives cannot be decreased when lockdrop in process
    if env.block.time.seconds() > config.init_timestamp
        && incentives_share < pool_info.incentives_share
    {
        return Err(StdError::generic_err(
            "Lockdrop in process. Incentives cannot be decreased for any pool",
        ));
    }

    // update total incentives
    state.total_incentives_share =
        state.total_incentives_share - pool_info.incentives_share + incentives_share;

    // Update Pool Incentives
    pool_info.incentives_share = incentives_share;

    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;
    STATE.save(deps.storage, &state)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "update_pool"),
        attr("terraswap_lp_token", terraswap_lp_token),
        attr("set_incentives_share", incentives_share.to_string()),
    ]))
}

/// @dev Admin function to enable ASTRO Claims by users. Called along-with Bootstrap Auction contract's LP Pool provide liquidity tx
pub fn handle_enable_claims(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    // CHECK :: ONLY AUCTION CONTRACT CAN CALL THIS FUNCTION
    if let Some(auction) = config.auction_contract {
        if info.sender != auction {
            return Err(StdError::generic_err("Unauthorized"));
        }
    } else {
        return Err(StdError::generic_err("Auction contract hasn't been set!"));
    }

    // CHECK :: Have the deposit / withdraw windows concluded
    if env.block.time.seconds()
        < (config.init_timestamp + config.deposit_window + config.withdrawal_window)
    {
        return Err(StdError::generic_err(
            "Deposit / withdraw windows not closed yet",
        ));
    }

    // CHECK ::: Claims are only enabled once
    if state.are_claims_allowed {
        return Err(StdError::generic_err("Already allowed"));
    }
    state.are_claims_allowed = true;

    STATE.save(deps.storage, &state)?;
    Ok(Response::new().add_attribute("action", "allow_claims"))
}

/// @dev Admin function to migrate Liquidity from Terraswap to Astroport
/// @param terraswap_lp_token : Parameter to identify the pool
/// @param astroport_pool_address : Astroport Pool address to which the liquidity is to be migrated
pub fn handle_migrate_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
    astroport_pool_addr: String,
    slippage_tolerance: Option<Decimal>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    // Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    // CHECK :: may the liquidity be migrated or not ?
    if env.block.time.seconds()
        < config.init_timestamp + config.deposit_window + config.withdrawal_window
    {
        return Err(StdError::generic_err(
            "Deposit / Withdrawal windows not closed",
        ));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;
    let astroport_pool = deps.api.addr_validate(&astroport_pool_addr)?;

    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    // CHECK :: has the liquidity already been migrated or not ?
    if pool_info.migration_info.is_some() {
        return Err(StdError::generic_err("Liquidity already migrated"));
    }

    let mut cosmos_msgs: Vec<CosmosMsg> = vec![];

    let lp_balance: BalanceResponse = deps.querier.query_wasm_smart(
        &terraswap_lp_token,
        &Cw20QueryMsg::Balance {
            address: env.contract.address.to_string(),
        },
    )?;

    // COSMOS MSG :: WITHDRAW LIQUIDITY FROM TERRASWAP
    let msg = WasmMsg::Execute {
        contract_addr: terraswap_lp_token.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: pool_info.terraswap_pool.to_string(),
            msg: to_binary(&terraswap::pair::Cw20HookMsg::WithdrawLiquidity {})?,
            amount: lp_balance.balance,
        })?,
    };
    cosmos_msgs.push(msg.into());

    let terraswap_lp_info: terraswap::asset::PairInfo = deps.querier.query_wasm_smart(
        &pool_info.terraswap_pool,
        &terraswap::pair::QueryMsg::Pair {},
    )?;

    let mut assets = vec![];

    for asset_info in terraswap_lp_info.asset_infos.iter() {
        assets.push(terraswap::asset::Asset {
            amount: match &asset_info {
                terraswap::asset::AssetInfo::NativeToken { denom } => {
                    terraswap::querier::query_balance(
                        &deps.querier,
                        env.contract.address.clone(),
                        denom.clone(),
                    )?
                }
                terraswap::asset::AssetInfo::Token { contract_addr } => {
                    terraswap::querier::query_token_balance(
                        &deps.querier,
                        deps.api.addr_validate(contract_addr)?,
                        env.contract.address.clone(),
                    )?
                }
            },
            info: asset_info.to_owned(),
        })
    }

    // COSMOS MSG :: CALLBACK AFTER LIQUIDITY WITHDRAWAL
    let update_state_msg = CallbackMsg::WithdrawLiquidityFromTerraswapCallback {
        terraswap_lp_token: terraswap_lp_token.clone(),
        astroport_pool: astroport_pool.clone(),
        prev_assets: assets.try_into().unwrap(),
        slippage_tolerance,
    }
    .to_cosmos_msg(&env)?;
    cosmos_msgs.push(update_state_msg);

    let astroport_lp_token = {
        let msg = astroport::pair::QueryMsg::Pair {};
        let res: astroport::asset::PairInfo =
            deps.querier.query_wasm_smart(&astroport_pool, &msg)?;
        res.liquidity_token
    };

    pool_info.migration_info = Some(MigrationInfo {
        astroport_lp_token,
        terraswap_migrated_amount: lp_balance.balance,
    });
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

    Ok(Response::new().add_messages(cosmos_msgs))
}

/// @dev Function to stake one of the supported LP Tokens with the Generator contract
/// @params terraswap_lp_token : Pool's terraswap LP token address whose Astroport LP tokens are to be staked
pub fn handle_stake_lp_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    let mut cosmos_msgs = vec![];

    // CHECK ::: Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;

    // CHECK ::: Is LP Token Pool supported or not ?
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    let MigrationInfo {
        astroport_lp_token, ..
    } = pool_info
        .migration_info
        .as_ref()
        .expect("Terraswap liquidity hasn't migrated yet!");

    let amount = {
        let res: BalanceResponse = deps.querier.query_wasm_smart(
            astroport_lp_token,
            &Cw20QueryMsg::Balance {
                address: env.contract.address.to_string(),
            },
        )?;
        res.balance
    };

    let generator = config.generator.expect("Generator address hasn't set yet!");

    cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: astroport_lp_token.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::IncreaseAllowance {
            spender: generator.to_string(),
            amount,
            expires: Some(cw20::Expiration::AtHeight(env.block.height + 1u64)),
        })?,
    }));

    cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: astroport_lp_token.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: generator.to_string(),
            msg: to_binary(&astroport::generator::Cw20HookMsg::Deposit {})?,
            amount,
        })?,
    }));

    // UPDATE STATE & SAVE
    pool_info.is_staked = true;
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

    Ok(Response::new()
        .add_messages(cosmos_msgs)
        .add_attributes(vec![
            attr("action", "stake_to_generator"),
            attr("terraswap_lp_token", terraswap_lp_token),
            attr("astroport_lp_amount", amount),
        ]))
}

/// @dev ReceiveCW20 Hook function to increase Lockup position size when any of the supported LP Tokens are sent to the contract by the user
/// @param user_address : User which sent the following LP token
/// @param duration : Number of weeks the LP token is locked for (lockup period begins post the withdrawal window closure)
/// @param amount : Number of LP tokens sent by the user
pub fn handle_increase_lockup(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    user_address: Addr,
    duration: u64,
    amount: Uint128,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let terraswap_lp_token = info.sender;

    // CHECK ::: LP Token supported or not ?
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    let mut user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    // CHECK :: Lockdrop deposit window open
    let current_time = env.block.time.seconds();
    if current_time < config.init_timestamp
        || current_time >= config.init_timestamp + config.deposit_window
    {
        return Err(StdError::generic_err("Deposit window closed"));
    }

    // CHECK :: Valid Lockup Duration
    if duration > config.max_lock_duration || duration < config.min_lock_duration {
        return Err(StdError::generic_err(format!(
            "Lockup duration needs to be between {} and {}",
            config.min_lock_duration, config.max_lock_duration
        )));
    }

    pool_info.weighted_amount += calculate_weight(amount, duration, &config);
    pool_info.terraswap_amount_in_lockups += amount;

    let lockup_key = (&terraswap_lp_token, &user_address, U64Key::new(duration));

    LOCKUP_INFO.update::<_, StdError>(deps.storage, lockup_key, |li| {
        if let Some(mut li) = li {
            li.lp_units_locked = li.lp_units_locked.checked_add(amount)?;
            Ok(li)
        } else {
            // Check :: Users cannot have more than max allowed number of lockup positions
            if config.max_positions_per_user == user_info.lockup_positions_index {
                return Err(StdError::generic_err(format!(
                    "Users can only have max {} lockup positions",
                    config.max_positions_per_user
                )));
            }
            // Update number of lockup positions the user is having
            user_info.lockup_positions_index += 1;

            Ok(LockupInfo {
                lp_units_locked: amount,
                astroport_lp_transferred: None,
                astro_rewards: Uint128::zero(),
                unlock_timestamp: config.init_timestamp
                    + config.deposit_window
                    + config.withdrawal_window
                    + (duration * SECONDS_PER_WEEK),
                generator_astro_debt: Uint128::zero(),
                generator_proxy_debt: Uint128::zero(),
                withdrawal_flag: false,
            })
        }
    })?;

    // SAVE UPDATED STATE
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;
    USER_INFO.save(deps.storage, &user_address, &user_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "increase_lockup_position"),
        attr("terraswap_lp_token", terraswap_lp_token),
        attr("user", user_address),
        attr("duration", duration.to_string()),
        attr("amount", amount),
    ]))
}

/// @dev Function to withdraw LP Tokens from an existing Lockup position
/// @param terraswap_lp_token : Terraswap Lp token address to identify the LP pool against which withdrawal has to be made
/// @param duration : Duration of the lockup position from which withdrawal is to be made
/// @param amount : Number of LP tokens to be withdrawn
pub fn handle_withdraw_from_lockup(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
    duration: u64,
    amount: Uint128,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    // CHECK :: Valid Withdraw Amount
    if amount.is_zero() {
        return Err(StdError::generic_err("Invalid withdrawal request"));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;

    // CHECK ::: LP Token supported or not ?
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    // Retrieve Lockup position
    let user_address = info.sender;
    let lockup_key = (&terraswap_lp_token, &user_address, U64Key::new(duration));
    let mut lockup_info = LOCKUP_INFO.load(deps.storage, lockup_key.clone())?;

    // CHECK :: Has user already withdrawn LP tokens once post the deposit window closure state
    if lockup_info.withdrawal_flag {
        return Err(StdError::generic_err(
            "Withdrawal already happened. No more withdrawals accepted",
        ));
    }

    // Check :: Amount should be within the allowed withdrawal limit bounds
    let max_withdrawal_percent =
        calculate_max_withdrawal_percent_allowed(env.block.time.seconds(), &config);
    let max_withdrawal_allowed = lockup_info.lp_units_locked * max_withdrawal_percent;
    if amount > max_withdrawal_allowed {
        return Err(StdError::generic_err(format!(
            "Amount exceeds maximum allowed withdrawal limit of {}",
            max_withdrawal_allowed
        )));
    }

    // Update withdrawal flag after the deposit window
    if env.block.time.seconds() >= config.init_timestamp + config.deposit_window {
        lockup_info.withdrawal_flag = true;
    }

    // STATE :: RETRIEVE --> UPDATE
    lockup_info.lp_units_locked -= amount;
    pool_info.weighted_amount -= calculate_weight(amount, duration, &config);
    pool_info.terraswap_amount_in_lockups -= amount;

    // Remove Lockup position from the list of user positions if Lp_Locked balance == 0
    if lockup_info.lp_units_locked.is_zero() {
        LOCKUP_INFO.remove(deps.storage, lockup_key);
        // decrement number of user's lockup positions
        let mut user_info = USER_INFO
            .may_load(deps.storage, &user_address)?
            .unwrap_or_default();
        user_info.lockup_positions_index -= 1;
        USER_INFO.save(deps.storage, &user_address, &user_info)?;
    } else {
        LOCKUP_INFO.save(deps.storage, lockup_key, &lockup_info)?;
    }

    // SAVE Updated States
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

    // COSMOS_MSG ::TRANSFER WITHDRAWN LP Tokens
    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: terraswap_lp_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: user_address.to_string(),
            amount,
        })?,
        funds: vec![],
    });

    Ok(Response::new().add_message(msg).add_attributes(vec![
        attr("action", "withdraw_from_lockup"),
        attr("terraswap_lp_token", terraswap_lp_token),
        attr("user_address", user_address),
        attr("duration", duration.to_string()),
        attr("amount", amount),
    ]))
}

// @dev Function to delegate part of the ASTRO rewards to be used for LP Bootstrapping via auction
/// @param amount : Number of ASTRO to delegate
pub fn handle_delegate_astro_to_auction(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    let user_address = info.sender;

    // CHECK :: Have the deposit / withdraw windows concluded
    if env.block.time.seconds()
        < (config.init_timestamp + config.deposit_window + config.withdrawal_window)
    {
        return Err(StdError::generic_err(
            "Deposit / withdraw windows not closed yet",
        ));
    }

    // CHECK :: Can users withdraw their ASTRO tokens ? -> if so, then delegation is no longer allowed
    if state.are_claims_allowed {
        return Err(StdError::generic_err("Delegation window over"));
    }

    let mut user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    // If user's total ASTRO rewards == 0 :: We update all of the user's lockup positions to calculate ASTRO rewards and for each alongwith their equivalent Astroport LP Shares
    if user_info.total_astro_rewards == Uint128::zero() {
        user_info.total_astro_rewards = update_user_lockup_positions_and_calc_rewards(
            deps.branch(),
            &config,
            &state,
            &user_address,
        )?;
    }

    // CHECK :: ASTRO to delegate cannot exceed user's unclaimed ASTRO balance
    let max_delegable_astro = user_info
        .total_astro_rewards
        .checked_sub(user_info.delegated_astro_rewards)?;

    if amount > max_delegable_astro {
        return Err(StdError::generic_err(format!("ASTRO to delegate cannot exceed user's unclaimed ASTRO balance. ASTRO to delegate = {}, Max delegable ASTRO = {}. ", amount, max_delegable_astro)));
    }

    // UPDATE STATE
    user_info.delegated_astro_rewards += amount;
    state.total_astro_delegated += amount;

    // SAVE UPDATED STATE
    STATE.save(deps.storage, &state)?;
    USER_INFO.save(deps.storage, &user_address, &user_info)?;

    // COSMOS_MSG ::Delegate ASTRO to the LP Bootstrapping via Auction contract
    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config
            .astro_token
            .expect("Astro token contract hasn't been set yet!")
            .to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config
                .auction_contract
                .expect("Auction contract hasn't been set yet!")
                .to_string(),
            msg: to_binary(&DelegateAstroTokens {
                user_address: user_address.to_string(),
            })?,
            amount,
        })?,
    });

    Ok(Response::new().add_message(msg).add_attributes(vec![
        attr("action", "delegate_astro_to_auction"),
        attr("user_address", user_address),
        attr("amount", amount),
    ]))
}

/// @dev Function to claim user Rewards for a particular Lockup position
/// @param terraswap_lp_token : Terraswap LP token to identify the LP pool whose Token is locked in the lockup position
/// @param duration : Lockup duration (number of weeks)
/// @param @withdraw_lp_stake : Boolean value indicating if the LP tokens are to be withdrawn or not
pub fn handle_claim_rewards_and_unlock_for_lockup(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    terraswap_lp_token: String,
    duration: u64,
    withdraw_lp_stake: bool,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;

    if !state.are_claims_allowed {
        return Err(StdError::generic_err("Reward claim not allowed"));
    }

    if env.block.time.seconds()
        < config.init_timestamp + config.deposit_window + config.withdrawal_window
    {
        return Err(StdError::generic_err(
            "Deposit / withdraw windows are still open",
        ));
    }

    let config = CONFIG.load(deps.storage)?;
    let user_address = info.sender;

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;

    // CHECK ::: Is LP Token Pool supported or not ?
    let pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    let mut user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    // If user's total ASTRO rewards == 0 :: We update all of the user's lockup positions to calculate ASTRO rewards and for each alongwith their equivalent Astroport LP Shares
    if user_info.total_astro_rewards == Uint128::zero() {
        user_info.total_astro_rewards = update_user_lockup_positions_and_calc_rewards(
            deps.branch(),
            &config,
            &state,
            &user_address,
        )?;
    }

    USER_INFO.save(deps.storage, &user_address, &user_info)?;

    // Check is there lockup or not ?
    let lockup_key = (&terraswap_lp_token, &user_address, U64Key::new(duration));
    let lockup_info = LOCKUP_INFO.load(deps.storage, lockup_key.clone())?;

    // CHECK :: Can the Lockup position be unlocked or not ?
    if withdraw_lp_stake && env.block.time.seconds() < lockup_info.unlock_timestamp {
        return Err(StdError::generic_err(format!(
            "{} seconds to unlock",
            lockup_info.unlock_timestamp - env.block.time.seconds()
        )));
    }

    if lockup_info.astroport_lp_transferred.is_some() {
        return Err(StdError::generic_err(
            "Astro LP Tokens have already been claimed!",
        ));
    }

    let mut cosmos_msgs = vec![];

    if let Some(MigrationInfo {
        astroport_lp_token, ..
    }) = &pool_info.migration_info
    {
        if pool_info.is_staked {
            let generator = config
                .generator
                .expect("Generator should be set at this moment!");

            // QUERY :: Check if there are any pending staking rewards
            let pending_rewards: PendingTokenResponse = deps.querier.query_wasm_smart(
                &generator,
                &GenQueryMsg::PendingToken {
                    lp_token: astroport_lp_token.to_string(),
                    user: env.contract.address.to_string(),
                },
            )?;

            if !pending_rewards.pending.is_zero()
                || (pending_rewards.pending_on_proxy.is_some()
                    && !pending_rewards.pending_on_proxy.unwrap().is_zero())
            {
                let rwi: RewardInfoResponse = deps.querier.query_wasm_smart(
                    &generator,
                    &GenQueryMsg::RewardInfo {
                        lp_token: astroport_lp_token.to_string(),
                    },
                )?;

                let astro_balance = {
                    let res: BalanceResponse = deps.querier.query_wasm_smart(
                        rwi.base_reward_token,
                        &Cw20QueryMsg::Balance {
                            address: env.contract.address.to_string(),
                        },
                    )?;
                    res.balance
                };

                let proxy_reward_balance = match rwi.proxy_reward_token {
                    Some(proxy_reward_token) => {
                        let res: BalanceResponse = deps.querier.query_wasm_smart(
                            proxy_reward_token,
                            &Cw20QueryMsg::Balance {
                                address: env.contract.address.to_string(),
                            },
                        )?;
                        Some(res.balance)
                    }
                    None => None,
                };

                cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: generator.to_string(),
                    funds: vec![],
                    msg: to_binary(&GenExecuteMsg::Withdraw {
                        lp_token: astroport_lp_token.to_string(),
                        amount: Uint128::zero(),
                    })?,
                }));

                cosmos_msgs.push(
                    CallbackMsg::UpdatePoolOnDualRewardsClaim {
                        terraswap_lp_token: terraswap_lp_token.clone(),
                        prev_astro_balance: astro_balance,
                        prev_proxy_reward_balance: proxy_reward_balance,
                    }
                    .to_cosmos_msg(&env)?,
                );
            }
        } else if user_info.astro_transferred && !withdraw_lp_stake {
            return Err(StdError::generic_err("No rewards available to claim!"));
        }

        // claim asset rewards if they support it
        if withdraw_lp_stake && pool_info.has_asset_rewards {
            cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: env.contract.address.to_string(),
                funds: vec![],
                msg: to_binary(&ExecuteMsg::ClaimAssetReward {
                    recipient: Some(user_address.to_string()),
                    terraswap_lp_token: terraswap_lp_token.to_string(),
                    duration,
                })?,
            }));
        };
    }

    cosmos_msgs.push(
        CallbackMsg::WithdrawUserLockupRewardsCallback {
            terraswap_lp_token,
            user_address,
            duration,
            withdraw_lp_stake,
        }
        .to_cosmos_msg(&env)?,
    );

    Ok(Response::new().add_messages(cosmos_msgs))
}

/// ## Description
/// Collects assets reward from LP and distribute reward to user if all requirements are met.
/// Otherwise returns [`StdError`].
fn handle_claim_asset_reward(
    deps: Deps,
    env: Env,
    user_address: Addr,
    recipient: Addr,
    terraswap_lp_token: String,
    lock_duration: u64,
) -> StdResult<Response> {
    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;
    let pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    if !pool_info.has_asset_rewards {
        return Err(StdError::generic_err("This pool does not have rewards"));
    }

    let migration_info = pool_info
        .migration_info
        .ok_or_else(|| StdError::generic_err("The pool was not migrated to astroport"))?;
    let pool_claim_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: migration_info.astroport_lp_token.to_string(),
        msg: to_binary(&astroport::pair_stable_bluna::ExecuteMsg::ClaimReward { receiver: None })?,
        funds: vec![],
    });

    let previous_balance = astroport::querier::query_balance(
        &deps.querier,
        env.contract.address.clone(),
        "uusd".to_string(),
    )?;

    let distribute_callback_msg = CallbackMsg::DistributeAssetReward {
        previous_balance,
        terraswap_lp_token,
        user_address,
        recipient,
        lock_duration,
    }
    .to_cosmos_msg(&env)?;

    Ok(Response::default().add_messages(vec![pool_claim_msg, distribute_callback_msg]))
}

/// ## Description
/// Sets `enable` flag for liquidity pool
fn handle_toggle_rewards(
    deps: DepsMut,
    info: MessageInfo,
    terraswap_lp_token: String,
    enable: bool,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    // CHECK ::: Only owner can call this function
    if info.sender != config.owner {
        return Err(StdError::generic_err("Unauthorized"));
    }

    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;
    ASSET_POOLS
        .update(deps.storage, &terraswap_lp_token, |pool_info_opt| {
            let mut pool_info =
                pool_info_opt.ok_or_else(|| StdError::generic_err("Pool was not found"))?;
            pool_info.has_asset_rewards = enable;
            Ok(pool_info)
        })
        .map(|pool_info| {
            Response::default().add_attributes(vec![
                ("action", "toggle_pool_rewards"),
                ("lp_address", pool_info.terraswap_pool.as_str()),
                (
                    "has_asset_rewards",
                    &pool_info.has_asset_rewards.to_string(),
                ),
            ])
        })
}

//----------------------------------------------------------------------------------------
// Callback Functions
//----------------------------------------------------------------------------------------

/// @dev CALLBACK Function to update contract state after dual staking rewards are claimed from the generator contract
/// @param terraswap_lp_token : Pool identifier to identify the LP pool whose rewards have been claimed
/// @param prev_astro_balance : Contract's ASTRO token balance before claim
/// @param prev_dual_reward_balance : Contract's Generator Proxy reward token balance before claim
pub fn update_pool_on_dual_rewards_claim(
    deps: DepsMut,
    env: Env,
    terraswap_lp_token: Addr,
    prev_astro_balance: Uint128,
    prev_proxy_reward_balance: Option<Uint128>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;

    let generator = config.generator.expect("Generator hasn't been set yet!");
    let MigrationInfo {
        astroport_lp_token, ..
    } = pool_info
        .migration_info
        .as_ref()
        .expect("Pool should be migrated!");

    let rwi: RewardInfoResponse = deps.querier.query_wasm_smart(
        &generator,
        &GenQueryMsg::RewardInfo {
            lp_token: astroport_lp_token.to_string(),
        },
    )?;

    let lp_balance: Uint128 = deps.querier.query_wasm_smart(
        &generator,
        &GenQueryMsg::Deposit {
            lp_token: astroport_lp_token.to_string(),
            user: env.contract.address.to_string(),
        },
    )?;

    let base_reward_received;
    // Increment claimed Astro rewards per LP share
    pool_info.generator_astro_per_share = pool_info.generator_astro_per_share + {
        let res: BalanceResponse = deps.querier.query_wasm_smart(
            rwi.base_reward_token,
            &Cw20QueryMsg::Balance {
                address: env.contract.address.to_string(),
            },
        )?;
        base_reward_received = res.balance - prev_astro_balance;
        Decimal::from_ratio(base_reward_received, lp_balance)
    };

    // Increment claimed Proxy rewards per LP share
    let mut proxy_reward_received = Uint128::zero();
    pool_info.generator_proxy_per_share = pool_info.generator_proxy_per_share + {
        match rwi.proxy_reward_token {
            Some(proxy_reward_token) => {
                let res: BalanceResponse = deps.querier.query_wasm_smart(
                    proxy_reward_token,
                    &Cw20QueryMsg::Balance {
                        address: env.contract.address.to_string(),
                    },
                )?;
                proxy_reward_received = res.balance
                    - prev_proxy_reward_balance.expect("Should be passed into this function!");
                Decimal::from_ratio(proxy_reward_received, lp_balance)
            }
            None => Decimal::zero(),
        }
    };

    // SAVE UPDATED STATE OF THE POOL
    ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "update_generator_dual_rewards"),
        attr("terraswap_lp_token", terraswap_lp_token),
        attr("astro_reward_received", base_reward_received),
        attr("proxy_reward_received", proxy_reward_received),
        attr(
            "generator_astro_per_share",
            pool_info.generator_astro_per_share.to_string(),
        ),
        attr(
            "generator_proxy_per_share",
            pool_info.generator_proxy_per_share.to_string(),
        ),
    ]))
}

/// @dev CALLBACK Function to withdraw user rewards and LP Tokens after claims / unlocks
/// @param terraswap_lp_token : Pool identifier to identify the LP pool
/// @param user_address : User address who is claiming the rewards / unlocking his lockup position
/// @param duration : Duration of the lockup for which rewards have been claimed / position unlocked
/// @param withdraw_lp_stake : Boolean value indicating if the ASTRO LP Tokens are to be sent to the user or not
pub fn callback_withdraw_user_rewards_for_lockup_optional_withdraw(
    deps: DepsMut,
    env: Env,
    terraswap_lp_token: Addr,
    user_address: Addr,
    duration: u64,
    withdraw_lp_stake: bool,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    let lockup_key = (&terraswap_lp_token, &user_address, U64Key::new(duration));
    let mut lockup_info = LOCKUP_INFO.load(deps.storage, lockup_key.clone())?;

    let mut user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    let mut cosmos_msgs = vec![];
    let mut attributes = vec![
        attr("action", "withdraw_rewards_and_or_unlock"),
        attr("terraswap_lp_token", &terraswap_lp_token),
        attr("user_address", &user_address),
        attr("duration", duration.to_string()),
    ];

    if let Some(MigrationInfo {
        astroport_lp_token, ..
    }) = &pool_info.migration_info
    {
        // Calculate Astro LP share for the lockup position
        let astroport_lp_amount: Uint128 = {
            let balance: Uint128 = if pool_info.is_staked {
                deps.querier.query_wasm_smart(
                    &config
                        .generator
                        .as_ref()
                        .expect("Should be set!")
                        .to_string(),
                    &GenQueryMsg::Deposit {
                        lp_token: astroport_lp_token.to_string(),
                        user: env.contract.address.to_string(),
                    },
                )?
            } else {
                let res: BalanceResponse = deps.querier.query_wasm_smart(
                    astroport_lp_token,
                    &Cw20QueryMsg::Balance {
                        address: env.contract.address.to_string(),
                    },
                )?;
                res.balance
            };
            (lockup_info.lp_units_locked.full_mul(balance)
                / Uint256::from(pool_info.terraswap_amount_in_lockups))
            .try_into()?
        };

        // If Astro LP tokens are staked with Astro generator
        if pool_info.is_staked {
            let generator = config.generator.expect("Generator should be set");

            let rwi: RewardInfoResponse = deps.querier.query_wasm_smart(
                &generator,
                &GenQueryMsg::RewardInfo {
                    lp_token: astroport_lp_token.to_string(),
                },
            )?;

            // Calculate claimable Astro staking rewards for this lockup
            let total_lockup_astro_rewards =
                pool_info.generator_astro_per_share * astroport_lp_amount;
            let pending_astro_rewards =
                total_lockup_astro_rewards - lockup_info.generator_astro_debt;
            lockup_info.generator_astro_debt = total_lockup_astro_rewards;

            // If claimable Astro staking rewards > 0, claim them
            if pending_astro_rewards > Uint128::zero() {
                cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: rwi.base_reward_token.to_string(),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: user_address.to_string(),
                        amount: pending_astro_rewards,
                    })?,
                }));
            }
            attributes.push(attr("generator_astro_reward", pending_astro_rewards));

            // If this is a void transaction (no state change), then return error.
            // Void tx scenario = ASTRO already claimed, 0 pending ASTRO staking reward, no proxy rewards, not unlocking LP tokens in this tx
            if !withdraw_lp_stake
                && user_info.astro_transferred
                && pending_astro_rewards == Uint128::zero()
                && rwi.proxy_reward_token.is_none()
            {
                return Err(StdError::generic_err("No rewards available to claim!"));
            }

            // If this LP token is getting dual incentives
            if let Some(proxy_reward_token) = rwi.proxy_reward_token {
                // Calculate claimable proxy staking rewards for this lockup
                let total_lockup_proxy_rewards =
                    pool_info.generator_proxy_per_share * astroport_lp_amount;
                let pending_proxy_rewards =
                    total_lockup_proxy_rewards - lockup_info.generator_proxy_debt;
                lockup_info.generator_proxy_debt = total_lockup_proxy_rewards;

                // If this is a void transaction (no state change), then return error.
                // Void tx scenario = ASTRO already claimed, 0 pending ASTRO staking reward, 0 pending proxy rewards, not unlocking LP tokens in this tx
                if !withdraw_lp_stake
                    && user_info.astro_transferred
                    && pending_astro_rewards == Uint128::zero()
                    && pending_proxy_rewards == Uint128::zero()
                {
                    return Err(StdError::generic_err("No rewards available to claim!"));
                }

                // If claimable proxy staking rewards > 0, claim them
                if pending_proxy_rewards > Uint128::zero() {
                    cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: proxy_reward_token.to_string(),
                        funds: vec![],
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            recipient: user_address.to_string(),
                            amount: pending_proxy_rewards,
                        })?,
                    }));
                }
                attributes.push(attr("generator_proxy_reward", pending_proxy_rewards));
            }

            //  COSMOSMSG :: If LP Tokens are staked, we unstake the amount which needs to be returned to the user
            if withdraw_lp_stake {
                cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: generator.to_string(),
                    funds: vec![],
                    msg: to_binary(&GenExecuteMsg::Withdraw {
                        lp_token: astroport_lp_token.to_string(),
                        amount: astroport_lp_amount,
                    })?,
                }));
            }
        }

        if withdraw_lp_stake {
            // COSMOSMSG :: Returns LP units locked by the user in the current lockup position
            cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: astroport_lp_token.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: user_address.to_string(),
                    amount: astroport_lp_amount,
                })?,
                funds: vec![],
            }));
            pool_info.terraswap_amount_in_lockups -= lockup_info.lp_units_locked;
            ASSET_POOLS.save(deps.storage, &terraswap_lp_token, &pool_info)?;

            attributes.push(attr("astroport_lp_unlocked", astroport_lp_amount));
            lockup_info.astroport_lp_transferred = Some(astroport_lp_amount);
        }
        LOCKUP_INFO.save(deps.storage, lockup_key, &lockup_info)?;
    } else if withdraw_lp_stake {
        return Err(StdError::generic_err("Pool should be migrated!"));
    }

    // Transfers claimable one time ASTRO rewards to the user that the user gets for all his lock
    if let Some(astro_token) = &config.astro_token {
        if !user_info.astro_transferred {
            // Calculating how much Astro user can claim (from total one time reward)
            let total_claimable_astro_rewards = user_info
                .total_astro_rewards
                .checked_sub(user_info.delegated_astro_rewards)?;
            if total_claimable_astro_rewards > Uint128::zero() {
                cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: astro_token.to_string(),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: user_address.to_string(),
                        amount: total_claimable_astro_rewards,
                    })?,
                }));
            }
            user_info.astro_transferred = true;
            attributes.push(attr(
                "total_claimable_astro_reward",
                total_claimable_astro_rewards,
            ));
            USER_INFO.save(deps.storage, &user_address, &user_info)?;
        }
    }

    Ok(Response::new()
        .add_messages(cosmos_msgs)
        .add_attributes(attributes))
}

/// @dev CALLBACK Function to deposit Liquidity in Astroport after its withdrawn from terraswap
/// @param terraswap_lp_token : Pool identifier to identify the LP pool
/// @param astroport_pool : Astroport Pool details to which the liquidity is to be migrated
/// @param prev_assets : balances of terraswap pool assets before liquidity was withdrawn
pub fn callback_deposit_liquidity_in_astroport(
    deps: DepsMut,
    env: Env,
    terraswap_lp_token: Addr,
    astroport_pool: Addr,
    prev_assets: [terraswap::asset::Asset; 2],
    slippage_tolerance: Option<Decimal>,
) -> StdResult<Response> {
    let mut cosmos_msgs = vec![];

    let mut assets = vec![];
    let mut coins = vec![];

    for prev_asset in prev_assets.iter() {
        match prev_asset.info.clone() {
            terraswap::asset::AssetInfo::NativeToken { denom } => {
                let mut new_asset = astroport::asset::Asset {
                    info: astroport::asset::AssetInfo::NativeToken {
                        denom: denom.clone(),
                    },
                    amount: terraswap::querier::query_balance(
                        &deps.querier,
                        env.contract.address.clone(),
                        denom.clone(),
                    )?
                    .checked_sub(prev_asset.amount)?,
                };

                new_asset.amount -= new_asset.compute_tax(&deps.querier)?;

                coins.push(Coin {
                    denom,
                    amount: new_asset.amount,
                });
                assets.push(new_asset);
            }
            terraswap::asset::AssetInfo::Token { contract_addr } => {
                let amount = terraswap::querier::query_token_balance(
                    &deps.querier,
                    deps.api.addr_validate(&contract_addr)?,
                    env.contract.address.clone(),
                )?
                .checked_sub(prev_asset.amount)?;

                cosmos_msgs.push(
                    WasmMsg::Execute {
                        contract_addr: contract_addr.to_string(),
                        funds: vec![],
                        msg: to_binary(&Cw20ExecuteMsg::IncreaseAllowance {
                            spender: astroport_pool.to_string(),
                            expires: Some(cw20::Expiration::AtHeight(env.block.height + 1u64)),
                            amount,
                        })?,
                    }
                    .into(),
                );

                assets.push(astroport::asset::Asset {
                    info: astroport::asset::AssetInfo::Token {
                        contract_addr: deps.api.addr_validate(&contract_addr)?,
                    },
                    amount,
                });
            }
        }
    }

    coins.sort_by(|a, b| a.denom.cmp(&b.denom));

    cosmos_msgs.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: astroport_pool.to_string(),
        funds: coins,
        msg: to_binary(&astroport::pair::ExecuteMsg::ProvideLiquidity {
            assets: assets.clone().try_into().unwrap(),
            slippage_tolerance,
            auto_stake: None,
            receiver: None,
        })?,
    }));

    Ok(Response::new()
        .add_messages(cosmos_msgs)
        .add_attributes(vec![
            attr("action", "migrate_liquidity_to_astroport"),
            attr("terraswap_lp_token", terraswap_lp_token),
            attr("astroport_pool", astroport_pool),
            attr("liquidity", format!("{}-{}", assets[0], assets[1])),
        ]))
}

fn callback_distribute_asset_reward(
    mut deps: DepsMut,
    env: Env,
    previous_balance: Uint128,
    recipient: Addr,
    terraswap_lp_token: Addr,
    user_address: Addr,
    lock_duration: u64,
) -> StdResult<Response> {
    let reward_balance =
        astroport::querier::query_balance(&deps.querier, env.contract.address, "uusd".to_string())?;
    let latest_reward_amount = reward_balance - previous_balance;

    let mut response = Response::new()
        .add_attribute("lockdrop_claimed_reward", latest_reward_amount)
        .add_attribute("user", user_address.clone());

    let pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    let total_asset_reward_path = TOTAL_ASSET_REWARD_INDEX.key(&terraswap_lp_token);
    let total_asset_reward_index = match total_asset_reward_path.may_load(deps.storage)? {
        Some(current_index) if !latest_reward_amount.is_zero() => {
            let val = current_index
                + Decimal256::from_ratio(
                    Uint256::from(latest_reward_amount),
                    pool_info.terraswap_amount_in_lockups,
                );
            total_asset_reward_path.save(deps.storage, &val)?;
            val
        }
        None => {
            let val = Decimal256::from_ratio(
                Uint256::from(latest_reward_amount),
                pool_info.terraswap_amount_in_lockups,
            );
            total_asset_reward_path.save(deps.storage, &val)?;
            val
        }
        Some(current_index) => current_index,
    };

    let lockup_key = (
        &terraswap_lp_token,
        &user_address,
        U64Key::new(lock_duration),
    );
    let mut user_reward = Uint128::zero();
    // get only lockups that have not yet been withdrawn
    let lockup_info_opt = LOCKUP_INFO
        .may_load(deps.storage, lockup_key.clone())?
        .filter(|lock_info| lock_info.astroport_lp_transferred.is_none());
    if let Some(lockup_info) = lockup_info_opt {
        let user_index_lp_path = USERS_ASSET_REWARD_INDEX.key(lockup_key);
        user_reward = calc_user_reward(
            deps.branch(),
            &user_index_lp_path,
            lockup_info.lp_units_locked,
            pool_info.terraswap_amount_in_lockups,
            total_asset_reward_index,
        )?;

        if !user_reward.is_zero() {
            response.messages.push(SubMsg::new(
                Asset {
                    info: AssetInfo::NativeToken {
                        denom: "uusd".to_string(),
                    },
                    amount: user_reward,
                }
                .into_msg(&deps.querier, recipient)?,
            ));
        }
    }

    Ok(response.add_attribute("sent_bluna_reward", user_reward))
}

// //----------------------------------------------------------------------------------------
// // Query Functions
// //----------------------------------------------------------------------------------------

/// @dev Returns the contract's configuration
pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(ConfigResponse {
        owner: config.owner,
        auction_contract: config.auction_contract,
        generator: config.generator,
        astro_token: config.astro_token,
        init_timestamp: config.init_timestamp,
        deposit_window: config.deposit_window,
        withdrawal_window: config.withdrawal_window,
        min_lock_duration: config.min_lock_duration,
        max_lock_duration: config.max_lock_duration,
        weekly_multiplier: config.weekly_multiplier,
        weekly_divider: config.weekly_divider,
        lockdrop_incentives: config.lockdrop_incentives,
        max_positions_per_user: config.max_positions_per_user,
    })
}

/// @dev Returns the contract's State
pub fn query_state(deps: Deps) -> StdResult<StateResponse> {
    let state: State = STATE.load(deps.storage)?;
    Ok(StateResponse {
        total_incentives_share: state.total_incentives_share,
        total_astro_delegated: state.total_astro_delegated,
        are_claims_allowed: state.are_claims_allowed,
        supported_pairs_list: ASSET_POOLS
            .keys(deps.storage, None, None, Order::Ascending)
            .map(|v| Addr::unchecked(String::from_utf8(v).expect("Addr deserialization error!")))
            .collect(),
    })
}

/// @dev Returns the pool's State
pub fn query_pool(deps: Deps, terraswap_lp_token: String) -> StdResult<PoolResponse> {
    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;
    let pool_info: PoolInfo = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    Ok(PoolResponse {
        terraswap_pool: pool_info.terraswap_pool,
        terraswap_amount_in_lockups: pool_info.terraswap_amount_in_lockups,
        migration_info: pool_info.migration_info,
        incentives_share: pool_info.incentives_share,
        weighted_amount: pool_info.weighted_amount,
        generator_astro_per_share: pool_info.generator_astro_per_share,
        generator_proxy_per_share: pool_info.generator_proxy_per_share,
        is_staked: pool_info.is_staked,
    })
}

/// @dev Returns summarized details regarding the user
pub fn query_user_info(deps: Deps, env: Env, user: String) -> StdResult<UserInfoResponse> {
    let user_address = deps.api.addr_validate(&user)?;
    let user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    let mut total_astro_rewards = Uint128::zero();
    let mut lockup_infos = vec![];

    let mut claimable_generator_astro_debt = Uint128::zero();
    for pool in ASSET_POOLS
        .keys(deps.storage, None, None, Order::Ascending)
        .map(|v| Addr::unchecked(String::from_utf8(v).expect("Addr deserialization error!")))
    {
        for duration in LOCKUP_INFO
            .prefix((&pool, &user_address))
            .keys(deps.storage, None, None, Order::Ascending)
            .map(|v| u64::from_be_bytes(v.try_into().expect("Duration deserialization error!")))
        {
            let lockup_info = query_lockup_info(deps, &env, &user, pool.to_string(), duration)?;
            total_astro_rewards += lockup_info.astro_rewards;
            claimable_generator_astro_debt += lockup_info.claimable_generator_astro_debt;
            lockup_infos.push(lockup_info);
        }
    }

    Ok(UserInfoResponse {
        total_astro_rewards,
        delegated_astro_rewards: user_info.delegated_astro_rewards,
        astro_transferred: user_info.astro_transferred,
        lockup_infos,
        claimable_generator_astro_debt,
        lockup_positions_index: user_info.lockup_positions_index,
    })
}

/// @dev Returns summarized details regarding the user with lockups list
pub fn query_user_info_with_lockups_list(
    deps: Deps,
    _env: Env,
    user: String,
) -> StdResult<UserInfoWithListResponse> {
    let user_address = deps.api.addr_validate(&user)?;
    let user_info = USER_INFO
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    let mut lockup_infos = vec![];

    for pool in ASSET_POOLS
        .keys(deps.storage, None, None, Order::Ascending)
        .map(|v| Addr::unchecked(String::from_utf8(v).expect("Addr deserialization error!")))
    {
        for duration in LOCKUP_INFO
            .prefix((&pool, &user_address))
            .keys(deps.storage, None, None, Order::Ascending)
            .map(|v| u64::from_be_bytes(v.try_into().expect("Duration deserialization error!")))
        {
            lockup_infos.push(LockUpInfoSummary {
                pool_address: pool.to_string(),
                duration,
            });
        }
    }

    Ok(UserInfoWithListResponse {
        total_astro_rewards: user_info.total_astro_rewards,
        delegated_astro_rewards: user_info.delegated_astro_rewards,
        astro_transferred: user_info.astro_transferred,
        lockup_infos,
        lockup_positions_index: user_info.lockup_positions_index,
    })
}

/// @dev Returns summarized details regarding the user
pub fn query_lockup_info(
    deps: Deps,
    env: &Env,
    user_address: &str,
    terraswap_lp_token: String,
    duration: u64,
) -> StdResult<LockUpInfoResponse> {
    let config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;
    let terraswap_lp_token = deps.api.addr_validate(&terraswap_lp_token)?;
    let user_address = deps.api.addr_validate(user_address)?;
    let lockup_key = (&terraswap_lp_token, &user_address, U64Key::new(duration));
    let mut pool_info = ASSET_POOLS.load(deps.storage, &terraswap_lp_token)?;
    let mut lockup_info = LOCKUP_INFO.load(deps.storage, lockup_key)?;

    let mut lockup_astroport_lp_units_opt: Option<Uint128> = None;
    let mut astroport_lp_token_opt: Option<Addr> = None;
    let mut claimable_generator_astro_debt = Uint128::zero();
    let mut claimable_generator_proxy_debt = Uint128::zero();
    if let Some(astroport_lp_transferred) = lockup_info.astroport_lp_transferred {
        lockup_astroport_lp_units_opt = Some(astroport_lp_transferred);
        astroport_lp_token_opt = pool_info.migration_info.map(|v| v.astroport_lp_token);
    } else if let Some(MigrationInfo {
        astroport_lp_token, ..
    }) = pool_info.migration_info.clone()
    {
        let pool_astroport_lp_units;
        let lockup_astroport_lp_units = {
            // Query Astro LP Tokens balance for the pool
            pool_astroport_lp_units = if pool_info.is_staked {
                deps.querier.query_wasm_smart(
                    &config
                        .generator
                        .as_ref()
                        .expect("Should be set!")
                        .to_string(),
                    &GenQueryMsg::Deposit {
                        lp_token: astroport_lp_token.to_string(),
                        user: env.contract.address.to_string(),
                    },
                )?
            } else {
                let res: BalanceResponse = deps.querier.query_wasm_smart(
                    &astroport_lp_token,
                    &Cw20QueryMsg::Balance {
                        address: env.contract.address.to_string(),
                    },
                )?;
                res.balance
            };
            // Calculate Lockup Astro LP shares
            (lockup_info
                .lp_units_locked
                .full_mul(pool_astroport_lp_units)
                / Uint256::from(pool_info.terraswap_amount_in_lockups))
            .try_into()?
        };
        lockup_astroport_lp_units_opt = Some(lockup_astroport_lp_units);
        astroport_lp_token_opt = Some(astroport_lp_token.clone());
        // If LP tokens are staked, calculate the rewards claimable by the user for this lockup position
        if pool_info.is_staked && !lockup_astroport_lp_units.is_zero() {
            let generator = config
                .generator
                .clone()
                .expect("Generator should be set at this moment!");

            // QUERY :: Check if there are any pending staking rewards
            let pending_rewards: PendingTokenResponse = deps.querier.query_wasm_smart(
                &generator,
                &GenQueryMsg::PendingToken {
                    lp_token: astroport_lp_token.to_string(),
                    user: env.contract.address.to_string(),
                },
            )?;

            // Calculate claimable Astro staking rewards for this lockup
            pool_info.generator_astro_per_share = pool_info.generator_astro_per_share
                + Decimal::from_ratio(pending_rewards.pending, pool_astroport_lp_units);

            let total_lockup_astro_rewards =
                pool_info.generator_astro_per_share * lockup_astroport_lp_units;
            claimable_generator_astro_debt =
                total_lockup_astro_rewards - lockup_info.generator_astro_debt;

            // Calculate claimable Proxy staking rewards for this lockup
            if pending_rewards.pending_on_proxy.is_some() {
                pool_info.generator_proxy_per_share = pool_info.generator_proxy_per_share
                    + Decimal::from_ratio(
                        pending_rewards.pending_on_proxy.unwrap(),
                        pool_astroport_lp_units,
                    );
                let total_lockup_proxy_rewards =
                    pool_info.generator_proxy_per_share * lockup_astroport_lp_units;
                claimable_generator_proxy_debt =
                    total_lockup_proxy_rewards - lockup_info.generator_proxy_debt;
            }
        }
    }
    // Calculate currently expected ASTRO Rewards if not finalized
    if lockup_info.astro_rewards == Uint128::zero() {
        let weighted_lockup_balance =
            calculate_weight(lockup_info.lp_units_locked, duration, &config);
        lockup_info.astro_rewards = calculate_astro_incentives_for_lockup(
            weighted_lockup_balance,
            pool_info.weighted_amount,
            pool_info.incentives_share,
            state.total_incentives_share,
            config.lockdrop_incentives,
        );
    }

    Ok(LockUpInfoResponse {
        terraswap_lp_token,
        lp_units_locked: lockup_info.lp_units_locked,
        withdrawal_flag: lockup_info.withdrawal_flag,
        astro_rewards: lockup_info.astro_rewards,
        generator_astro_debt: lockup_info.generator_astro_debt,
        claimable_generator_astro_debt,
        generator_proxy_debt: lockup_info.generator_proxy_debt,
        claimable_generator_proxy_debt,
        unlock_timestamp: lockup_info.unlock_timestamp,
        astroport_lp_units: lockup_astroport_lp_units_opt,
        astroport_lp_token: astroport_lp_token_opt,
        astroport_lp_transferred: lockup_info.astroport_lp_transferred,
        duration,
    })
}

//----------------------------------------------------------------------------------------
// HELPERS :: BOOLEANS & COMPUTATIONS (Rewards, Indexes etc)
//----------------------------------------------------------------------------------------

///  @dev Helper function to calculate maximum % of LP balances deposited that can be withdrawn
/// @params current_timestamp : Current block timestamp
/// @params config : Contract configuration
fn calculate_max_withdrawal_percent_allowed(current_timestamp: u64, config: &Config) -> Decimal {
    let withdrawal_cutoff_init_point = config.init_timestamp + config.deposit_window;

    // Deposit window :: 100% withdrawals allowed
    if current_timestamp < withdrawal_cutoff_init_point {
        return Decimal::from_ratio(100u32, 100u32);
    }

    let withdrawal_cutoff_second_point =
        withdrawal_cutoff_init_point + (config.withdrawal_window / 2u64);
    // Deposit window closed, 1st half of withdrawal window :: 50% withdrawals allowed
    if current_timestamp <= withdrawal_cutoff_second_point {
        return Decimal::from_ratio(50u32, 100u32);
    }

    // max withdrawal allowed decreasing linearly from 50% to 0% vs time elapsed
    let withdrawal_cutoff_final = withdrawal_cutoff_init_point + config.withdrawal_window;
    //  Deposit window closed, 2nd half of withdrawal window :: max withdrawal allowed decreases linearly from 50% to 0% vs time elapsed
    if current_timestamp < withdrawal_cutoff_final {
        let time_left = withdrawal_cutoff_final - current_timestamp;
        Decimal::from_ratio(
            50u64 * time_left,
            100u64 * (withdrawal_cutoff_final - withdrawal_cutoff_second_point),
        )
    }
    // Withdrawals not allowed
    else {
        Decimal::from_ratio(0u32, 100u32)
    }
}

/// @dev Helper function to calculate ASTRO rewards for a particular Lockup position
/// @params lockup_weighted_balance : Lockup position's weighted terraswap LP balance
/// @params total_weighted_amount : Total weighted terraswap LP balance of the Pool
/// @params pool_incentives_share : Share of total ASTRO incentives allocated to this pool
/// @params total_incentives_share: Calculated total incentives share for allocating among pools
/// @params total_lockdrop_incentives : Total ASTRO incentives to be distributed among Lockdrop participants
pub fn calculate_astro_incentives_for_lockup(
    lockup_weighted_balance: Uint256,
    total_weighted_amount: Uint256,
    pool_incentives_share: u64,
    total_incentives_share: u64,
    total_lockdrop_incentives: Uint128,
) -> Uint128 {
    if total_incentives_share == 0u64 || total_weighted_amount == Uint256::zero() {
        Uint128::zero()
    } else {
        (Decimal256::from_ratio(
            Uint256::from(pool_incentives_share) * lockup_weighted_balance,
            Uint256::from(total_incentives_share) * total_weighted_amount,
        ) * total_lockdrop_incentives.into())
        .try_into()
        .unwrap()
    }
}

/// @dev Helper function. Returns effective weight for the amount to be used for calculating lockdrop rewards
/// @params amount : Number of LP tokens
/// @params duration : Number of weeks
/// @config : Config with weekly multiplier and divider
fn calculate_weight(amount: Uint128, duration: u64, config: &Config) -> Uint256 {
    let lock_weight = Decimal256::one()
        + Decimal256::from_ratio(
            (duration - 1) * config.weekly_multiplier,
            config.weekly_divider,
        );
    lock_weight * amount.into()
}

/// ## Description
/// Calculates bLuna user reward according to his share in LP  
fn calc_user_reward(
    deps: DepsMut,
    user_index_lp_path: &Path<Decimal256>,
    user_lp_amount: Uint128,
    total_lp_amount: Uint128,
    total_reward_index: Decimal256,
) -> StdResult<Uint128> {
    if user_lp_amount.is_zero() || total_lp_amount.is_zero() {
        return Ok(Uint128::zero());
    }

    let to_distribute_index = match user_index_lp_path.may_load(deps.storage)? {
        None => total_reward_index,
        Some(last_user_bluna_reward_index) if last_user_bluna_reward_index < total_reward_index => {
            total_reward_index - last_user_bluna_reward_index
        }
        _ => return Ok(Uint128::zero()),
    };

    user_index_lp_path.save(deps.storage, &total_reward_index)?;

    (to_distribute_index * Uint256::from(user_lp_amount))
        .try_into()
        .map_err(Into::into)
}

//-----------------------------------------------------------
// HELPER FUNCTIONS :: UPDATE STATE
//-----------------------------------------------------------

/// @dev Function to calculate ASTRO rewards for each of the user position
/// @params configuration struct
/// @params user Info struct
/// Returns user's total ASTRO rewards
fn update_user_lockup_positions_and_calc_rewards(
    deps: DepsMut,
    config: &Config,
    state: &State,
    user_address: &Addr,
) -> StdResult<Uint128> {
    let mut total_astro_rewards = Uint128::zero();

    let mut keys: Vec<(Addr, u64)> = vec![];

    for pool_key in ASSET_POOLS
        .keys(deps.storage, None, None, Order::Ascending)
        .map(|v| Addr::unchecked(String::from_utf8(v).expect("Addr deserialization error!")))
    {
        for duration in LOCKUP_INFO
            .prefix((&pool_key, user_address))
            .keys(deps.storage, None, None, Order::Ascending)
            .map(|v| u64::from_be_bytes(v.try_into().expect("Duration deserialization error!")))
        {
            keys.push((pool_key.clone(), duration));
        }
    }
    for (pool, duration) in keys {
        let pool_info = ASSET_POOLS.load(deps.storage, &pool)?;
        let lockup_key = (&pool, user_address, U64Key::new(duration));
        let mut lockup_info = LOCKUP_INFO.load(deps.storage, lockup_key.clone())?;

        let lockup_astro_rewards: Uint128;

        if lockup_info.astro_rewards == Uint128::zero() {
            // Weighted lockup balance (using terraswap LP units to calculate as pool's total weighted balance is calculated on terraswap LP deposits summed over each deposit tx)
            let weighted_lockup_balance =
                calculate_weight(lockup_info.lp_units_locked, duration, config);

            // Calculate ASTRO Lockdrop rewards for the lockup position
            lockup_info.astro_rewards = calculate_astro_incentives_for_lockup(
                weighted_lockup_balance,
                pool_info.weighted_amount,
                pool_info.incentives_share,
                state.total_incentives_share,
                config.lockdrop_incentives,
            );

            LOCKUP_INFO.save(deps.storage, lockup_key, &lockup_info)?;
        };

        lockup_astro_rewards = lockup_info.astro_rewards;

        // Save updated Lockup state
        total_astro_rewards += lockup_astro_rewards;
    }

    Ok(total_astro_rewards)
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::mock_querier::mock_dependencies;
    use cosmwasm_std::testing::{mock_env, mock_info};
    use cosmwasm_std::{Attribute, Timestamp};

    #[test]
    fn bluna_rewards_claim() {
        let init_uusd_balance = Uint128::from(100u128);
        let mut deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: init_uusd_balance,
        }]);
        let owner = "owner";
        let mut env = mock_env();
        env.block.time = Timestamp::from_seconds(0);
        let lockdrop_instantiate_msg = InstantiateMsg {
            owner: Some(owner.to_string()),
            init_timestamp: 100_000,
            deposit_window: 10_000_000,
            withdrawal_window: 500_000,
            min_lock_duration: 1u64,
            max_lock_duration: 52u64,
            weekly_multiplier: 1u64,
            weekly_divider: 12u64,
            max_positions_per_user: 14,
        };
        instantiate(
            deps.as_mut(),
            env.clone(),
            mock_info(owner, &[]),
            lockdrop_instantiate_msg,
        )
        .unwrap();

        let user_addr = Addr::unchecked("user");
        let astroport_lp_token = Addr::unchecked("astro_lp_addr");
        let terraswap_lp_addr = Addr::unchecked("tswp_lp_token");
        let migration_info = MigrationInfo {
            terraswap_migrated_amount: Uint128::from(100_000000u128),
            astroport_lp_token: astroport_lp_token.clone(),
        };
        let pool_info = PoolInfo {
            terraswap_pool: Addr::unchecked(terraswap_lp_addr.clone()),
            terraswap_amount_in_lockups: Default::default(),
            migration_info: Some(migration_info),
            incentives_share: 0,
            weighted_amount: Default::default(),
            generator_astro_per_share: Default::default(),
            generator_proxy_per_share: Default::default(),
            is_staked: false,
            has_asset_rewards: false,
        };
        ASSET_POOLS
            .save(deps.as_mut().storage, &terraswap_lp_addr, &pool_info)
            .unwrap();

        let lock_duration = 10;
        // check the user cannot claim reward before rewards are enabled
        let res = handle_claim_asset_reward(
            deps.as_ref(),
            env.clone(),
            user_addr.clone(),
            user_addr.clone(),
            terraswap_lp_addr.to_string(),
            lock_duration,
        )
        .unwrap_err();
        assert_eq!(
            res.to_string(),
            "Generic error: This pool does not have rewards"
        );

        // enabling rewards
        handle_toggle_rewards(
            deps.as_mut(),
            mock_info("owner", &[]),
            terraswap_lp_addr.to_string(),
            true,
        )
        .unwrap();

        let res = handle_claim_asset_reward(
            deps.as_ref(),
            env.clone(),
            user_addr.clone(),
            user_addr.clone(),
            terraswap_lp_addr.to_string(),
            lock_duration,
        )
        .unwrap();

        // check dispatched messages
        if let CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr, msg, ..
        }) = &res.messages[0].msg
        {
            assert_eq!(contract_addr.to_owned(), astroport_lp_token.to_string());
            assert_eq!(
                from_binary::<astroport::pair_stable_bluna::ExecuteMsg>(&msg).unwrap(),
                astroport::pair_stable_bluna::ExecuteMsg::ClaimReward { receiver: None }
            )
        } else {
            panic!("Wrong message")
        }

        if let CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr, msg, ..
        }) = &res.messages[1].msg
        {
            assert_eq!(contract_addr.to_owned(), env.contract.address.to_string());
            let real_message = ExecuteMsg::Callback(CallbackMsg::DistributeAssetReward {
                terraswap_lp_token: terraswap_lp_addr,
                user_address: user_addr.clone(),
                recipient: user_addr,
                lock_duration: 10,
                previous_balance: init_uusd_balance,
            });
            assert_eq!(from_binary::<ExecuteMsg>(&msg).unwrap(), real_message);
        } else {
            panic!("Wrong message")
        }
    }

    #[test]
    fn check_calc_user_reward() {
        let mut deps = mock_dependencies(&[]);
        let terraswap_lp_token = Addr::unchecked("lp_token_addr");
        let total_lp_amount = Uint128::from(1000u128);
        // user1 with 10% share
        let user1 = Addr::unchecked("user1");
        let user1_path =
            USERS_ASSET_REWARD_INDEX.key((&terraswap_lp_token, &user1, U64Key::new(10)));
        let user1_lp_amount = Uint128::from(100u128);
        // user2 with 70% share
        let user2 = Addr::unchecked("user2");
        let user2_path =
            USERS_ASSET_REWARD_INDEX.key((&terraswap_lp_token, &user2, U64Key::new(10)));
        let user2_lp_amount = Uint128::from(700u128);
        // user3 with 20% share
        let user3 = Addr::unchecked("user3");
        let user3_path =
            USERS_ASSET_REWARD_INDEX.key((&terraswap_lp_token, &user3, U64Key::new(10)));
        let user3_lp_amount = Uint128::from(200u128);
        let mut total_reward_index = Decimal256::one();

        let res = calc_user_reward(
            deps.as_mut(),
            &user1_path,
            user1_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();

        assert_eq!(res.u128(), 100u128);

        // the user already received whole reward thus we get 0 here
        let res = calc_user_reward(
            deps.as_mut(),
            &user1_path,
            user1_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();
        assert_eq!(res.u128(), 0u128);

        let res = calc_user_reward(
            deps.as_mut(),
            &user2_path,
            user2_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();
        assert_eq!(res.u128(), 700u128);

        // emulating newly arrived rewards
        total_reward_index = total_reward_index + Decimal256::from_ratio(100u128, 1000u128);

        let res = calc_user_reward(
            deps.as_mut(),
            &user1_path,
            user1_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();

        assert_eq!(res.u128(), 10u128);

        // the user already received whole reward thus we get 0 here
        let res = calc_user_reward(
            deps.as_mut(),
            &user1_path,
            user1_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();
        assert_eq!(res.u128(), 0u128);

        let res = calc_user_reward(
            deps.as_mut(),
            &user2_path,
            user2_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();
        assert_eq!(res.u128(), 70u128);

        // this is the first time user3 receives reward
        let res = calc_user_reward(
            deps.as_mut(),
            &user3_path,
            user3_lp_amount,
            total_lp_amount,
            total_reward_index,
        )
        .unwrap();
        // 200 from the first distribution and 20 from the second one
        assert_eq!(res.u128(), 220u128);
    }

    #[test]
    fn check_distribute_asset_reward() {
        let mut uusd_balance = Uint128::from(100u128);
        let mut deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: uusd_balance,
        }]);
        let owner = "owner";
        let mut env = mock_env();
        env.block.time = Timestamp::from_seconds(0);
        let lockdrop_instantiate_msg = InstantiateMsg {
            owner: Some(owner.to_string()),
            init_timestamp: 100_000,
            deposit_window: 10_000_000,
            withdrawal_window: 500_000,
            min_lock_duration: 1u64,
            max_lock_duration: 52u64,
            weekly_multiplier: 1u64,
            weekly_divider: 12u64,
            max_positions_per_user: 14,
        };
        instantiate(
            deps.as_mut(),
            env.clone(),
            mock_info(owner, &[]),
            lockdrop_instantiate_msg,
        )
        .unwrap();

        let user_addr = Addr::unchecked("user");
        let lock_duration = 10;
        let astroport_lp_token = Addr::unchecked("astro_lp_addr");
        let terraswap_lp_addr = Addr::unchecked("tswp_lp_token");
        let migration_info = MigrationInfo {
            terraswap_migrated_amount: Uint128::from(100_000000u128),
            astroport_lp_token: astroport_lp_token.clone(),
        };
        let pool_info = PoolInfo {
            terraswap_pool: Addr::unchecked(terraswap_lp_addr.clone()),
            terraswap_amount_in_lockups: Uint128::from(1000u128),
            migration_info: Some(migration_info),
            incentives_share: 0,
            weighted_amount: Default::default(),
            generator_astro_per_share: Default::default(),
            generator_proxy_per_share: Default::default(),
            is_staked: false,
            has_asset_rewards: true,
        };
        ASSET_POOLS
            .save(deps.as_mut().storage, &terraswap_lp_addr, &pool_info)
            .unwrap();

        let lockup = LockupInfo {
            lp_units_locked: Uint128::from(100u128),
            astroport_lp_transferred: None,
            withdrawal_flag: false,
            astro_rewards: Default::default(),
            generator_astro_debt: Default::default(),
            generator_proxy_debt: Default::default(),
            unlock_timestamp: 0,
        };
        let lockup_key = (&terraswap_lp_addr, &user_addr, U64Key::new(lock_duration));
        LOCKUP_INFO
            .save(deps.as_mut().storage, lockup_key, &lockup)
            .unwrap();

        // let's try to receive reward for non-existent lockup
        let resp = callback_distribute_asset_reward(
            deps.as_mut(),
            env.clone(),
            uusd_balance,
            user_addr.clone(),
            terraswap_lp_addr.clone(),
            user_addr.clone(),
            100,
        )
        .unwrap();
        assert_eq!(resp.messages.len(), 0);
        assert_eq!(
            &resp.attributes[0],
            Attribute {
                key: "lockdrop_claimed_reward".to_string(),
                value: "0".to_string()
            }
        );
        assert_eq!(
            &resp.attributes[1],
            Attribute {
                key: "user".to_string(),
                value: "user".to_string()
            }
        );

        // emulating newly arrived rewards
        deps.querier.with_balance(&[(
            &env.contract.address.to_string(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: uusd_balance + Uint128::from(100u128),
            }],
        )]);

        let resp = callback_distribute_asset_reward(
            deps.as_mut(),
            env.clone(),
            uusd_balance,
            user_addr.clone(),
            terraswap_lp_addr.clone(),
            user_addr.clone(),
            lock_duration,
        )
        .unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(
            &resp.attributes[0],
            Attribute {
                key: "lockdrop_claimed_reward".to_string(),
                value: "100".to_string()
            }
        );
        assert_eq!(
            &resp.attributes[1],
            Attribute {
                key: "user".to_string(),
                value: "user".to_string()
            }
        );
        assert_eq!(
            &resp.attributes[2],
            Attribute {
                key: "sent_bluna_reward".to_string(),
                value: "10".to_string()
            }
        );

        uusd_balance += Uint128::from(90u128);

        // 90 ASTRO stays on the balance
        deps.querier.with_balance(&[(
            &env.contract.address.to_string(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: uusd_balance,
            }],
        )]);

        // the user already received reward
        let resp = callback_distribute_asset_reward(
            deps.as_mut(),
            env.clone(),
            uusd_balance,
            user_addr.clone(),
            terraswap_lp_addr.clone(),
            user_addr.clone(),
            lock_duration,
        )
        .unwrap();
        assert_eq!(resp.messages.len(), 0);
        assert_eq!(
            &resp.attributes[2],
            Attribute {
                key: "sent_bluna_reward".to_string(),
                value: "0".to_string()
            }
        );

        uusd_balance -= Uint128::from(10u128);

        // emulating newly arrived rewards
        deps.querier.with_balance(&[(
            &env.contract.address.to_string(),
            &[Coin {
                denom: "uusd".to_string(),
                amount: uusd_balance + Uint128::from(500u128),
            }],
        )]);
        // the user should receive rewards from the seconds distribution
        let resp = callback_distribute_asset_reward(
            deps.as_mut(),
            env.clone(),
            uusd_balance,
            user_addr.clone(),
            terraswap_lp_addr.clone(),
            user_addr.clone(),
            lock_duration,
        )
        .unwrap();
        assert_eq!(resp.messages.len(), 1);
        assert_eq!(
            &resp.attributes[2],
            Attribute {
                key: "sent_bluna_reward".to_string(),
                value: "50".to_string()
            }
        );
    }
}
