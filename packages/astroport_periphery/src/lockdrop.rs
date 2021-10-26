use cosmwasm_std::{
    to_binary, Addr, CosmosMsg, Decimal, Env, StdResult, Uint128, Uint256, WasmMsg,
};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    /// Account which can update config
    pub owner: Option<String>,
    /// Timestamp when Contract will start accepting LP Token deposits
    pub init_timestamp: u64,
    /// Number of seconds during which lockup deposits will be accepted
    pub deposit_window: u64,
    /// Withdrawal Window Length :: Post the deposit window
    pub withdrawal_window: u64,
    /// Min. no. of weeks allowed for lockup
    pub min_lock_duration: u64,
    /// Max. no. of weeks allowed for lockup
    pub max_lock_duration: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UpdateConfigMsg {
    /// Account which can update config
    pub owner: Option<String>,
    /// Astroport token address
    pub astro_token_address: Option<String>,
    /// Bootstrap Auction contract address
    pub auction_contract_address: Option<String>,
    /// Generator (Staking for dual rewards) contract address
    pub generator_address: Option<String>,
    /// Total ASTRO lockdrop incentives to be distributed among the users
    pub lockdrop_incentives: Option<Uint128>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    // Receive hook used to accept LP Token deposits
    Receive(Cw20ReceiveMsg),
    // ADMIN Function ::: To update configuration
    UpdateConfig {
        new_config: UpdateConfigMsg,
    },
    // Called by the bootstrap auction contract when liquidity is added to the
    // Pool to enable ASTRO withdrawals by users
    EnableClaims {},
    // ADMIN Function ::: Add new Pool (Only Terraswap Pools)
    InitializePool {
        terraswap_lp_token: String,
        incentives_share: u64,
    },
    // ADMIN Function ::: To set incentives_share for the Pool
    UpdatePool {
        terraswap_lp_token: String,
        incentives_share: u64,
    },
    // ADMIN Function ::: To transfer ASTRO Tokens which have been returned to force unlock LP positions
    TransferReturnedAstro {
        recepient: String,
        amount: Uint128,
    },
    // Function to facilitate LP Token withdrawals from lockups
    WithdrawFromLockup {
        terraswap_lp_token: String,
        duration: u64,
        amount: Uint128,
    },

    // ADMIN Function ::: To Migrate liquidity from terraswap to astroport
    MigrateLiquidity {
        terraswap_lp_token: String,
        astroport_pool_addr: String,
    },
    // // ADMIN Function ::: To stake LP Tokens with the generator contract
    StakeLpTokens {
        terraswap_lp_token: String,
    },
    // Delegate ASTRO to Bootstrap via auction contract
    DelegateAstroToAuction {
        amount: Uint128,
    },
    // Facilitates ASTRO reward withdrawal which have not been delegated to bootstrap auction
    ClaimRewardsForLockup {
        terraswap_lp_token: String,
        duration: u64,
    },
    // Unlocks a lockup position whose lockup duration has concluded
    UnlockPosition {
        terraswap_lp_token: String,
        duration: u64,
    },
    // Unlocks a lockup position whose lockup duration has not concluded. user needs to approve ASTRO Token to
    // be transferred by the lockdrop contract before calling this function
    ForceUnlockPosition {
        terraswap_lp_token: String,
        duration: u64,
    },
    /// Callbacks; only callable by the contract itself.
    Callback(CallbackMsg),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    /// Open a new user position or add to an existing position (Cw20ReceiveMsg)
    IncreaseLockup { duration: u64 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallbackMsg {
    UpdatePoolOnDualRewardsClaim {
        terraswap_lp_token: Addr,
        prev_astro_balance: Uint128,
        prev_proxy_reward_balance: Option<Uint128>,
    },
    WithdrawUserLockupRewardsCallback {
        terraswap_lp_token: Addr,
        user_address: Addr,
        duration: u64,
        withdraw_lp_stake: bool,
        force_unlock: bool,
    },
    WithdrawLiquidityFromTerraswapCallback {
        terraswap_lp_token: Addr,
        astroport_pool: Addr,
        prev_assets: [terraswap::asset::Asset; 2],
    },
}

// Modified from
// https://github.com/CosmWasm/cosmwasm-plus/blob/v0.2.3/packages/cw20/src/receiver.rs#L15
impl CallbackMsg {
    pub fn to_cosmos_msg(self, env: &Env) -> StdResult<CosmosMsg> {
        Ok(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_binary(&ExecuteMsg::Callback(self))?,
            funds: vec![],
        }))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    State {},
    Pool {
        terraswap_lp_token: String,
    },
    UserInfo {
        address: String,
    },
    LockUpInfo {
        user_address: String,
        terraswap_lp_token: String,
        duration: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    /// Account which can update config
    pub owner: Addr,
    /// ASTRO Token address
    pub astro_token: Option<Addr>,
    /// Bootstrap Auction contract address
    pub auction_contract: Option<Addr>,
    /// Generator (Staking for dual rewards) contract address
    pub generator: Option<Addr>,
    /// Timestamp when Contract will start accepting LP Token deposits
    pub init_timestamp: u64,
    /// Number of seconds during which lockup deposits will be accepted
    pub deposit_window: u64,
    /// Withdrawal Window Length :: Post the deposit window
    pub withdrawal_window: u64,
    /// Min. no. of weeks allowed for lockup
    pub min_lock_duration: u64,
    /// Max. no. of weeks allowed for lockup
    pub max_lock_duration: u64,
    /// Total ASTRO lockdrop incentives to be distributed among the users
    pub lockdrop_incentives: Option<Uint128>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    /// Total ASTRO incentives share
    pub total_incentives_share: u64,
    /// ASTRO Tokens delegated to the bootstrap auction contract
    pub total_astro_delegated: Uint128,
    /// ASTRO returned to forcefully unlock Lockup positions
    pub total_astro_returned_available: Uint128,
    /// Boolean value indicating if the user can withdraw thier ASTRO rewards or not
    pub are_claims_allowed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PoolResponse {
    pub terraswap_pool: Addr,
    pub terraswap_migrated_amount: Option<Uint128>,
    pub astroport_lp_token: Option<Addr>,
    pub astroport_pool: Option<Addr>,
    /// Share of total ASTRO incentives allocated to this pool
    pub incentives_share: u64,
    /// Weighted LP Token balance used to calculate ASTRO rewards a particular user can claim
    pub weighted_amount: Uint256,
    /// Ratio of ASTRO rewards accured to weighted_amount. Used to calculate ASTRO incentives accured by each user
    pub generator_astro_per_share: Decimal,
    /// Ratio of ASSET rewards accured to weighted. Used to calculate ASSET incentives accured by each user
    pub generator_proxy_per_share: Decimal,
    /// Boolean value indicating if the LP Tokens are staked with the Generator contract or not
    pub is_staked: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UserInfoResponse {
    /// Total ASTRO tokens user received as rewards for participation in the lockdrop
    pub total_astro_rewards: Uint128,
    /// Total ASTRO tokens user delegated to the LP bootstrap auction pool
    pub delegated_astro_rewards: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LockUpInfoResponse {
    /// Terraswap LP units locked by the user
    pub lp_units_locked: Uint128,
    /// Boolean value indicating if the user's has withdrawn funds post the only 1 withdrawal limit cutoff
    pub withdrawal_flag: bool,
    /// ASTRO tokens received as rewards for participation in the lockdrop
    pub astro_rewards: Option<Uint128>,
    /// ASTRO tokens transferred to user
    pub astro_transferred: bool,
    /// Generator ASTRO tokens loockup received as generator rewards
    pub generator_astro_debt: Uint128,
    /// Generator Proxy tokens lockup received as generator rewards
    pub generator_proxy_debt: Uint128,
    /// Timestamp beyond which this position can be unlocked
    pub unlock_timestamp: u64,
    /// User's Astroport LP units, calculated as lp_units_locked (terraswap) / total LP units locked (terraswap) * Astroport LP units minted post migration
    pub astroport_lp_units: Option<Uint128>,
}
