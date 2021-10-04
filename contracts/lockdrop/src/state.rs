use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

use cosmwasm_bignumber::{Decimal256, Uint256};
use astroport_periphery::helpers::zero_address;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CONFIG: Item<Config> = Item::new("config");
pub const STATE: Item<State> = Item::new("state");

pub const ASSET_POOLS: Map<&Addr, PoolInfo> = Map::new("lp_assets");
pub const USER_INFO: Map<&Addr, UserInfo> = Map::new("users");
pub const LOCKUP_INFO: Map<&[u8], LockupInfo> = Map::new("lockup_position");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Account who can update config
    pub owner: Addr,
    /// Bootstrap Auction contract address
    pub auction_contract_address: Addr,
    /// Generator (ASTRO-UST Staking) contract address
    pub generator_address: Addr,
    /// ASTRO Token address
    pub astro_token_address: Addr,
    /// Timestamp when Contract will start accepting LP Token deposits
    pub init_timestamp: u64,
    /// Deposit Window Length
    pub deposit_window: u64,
    /// Withdrawal Window Length :: Post the deposit window
    pub withdrawal_window: u64,
    /// Min. no. of weeks allowed for lockup
    pub min_lock_duration: u64,
    /// Max. no. of weeks allowed for lockup
    pub max_lock_duration: u64,
    /// Number of seconds per week
    pub seconds_per_week: u64,
    /// Lockdrop Reward multiplier
    pub weekly_multiplier: Decimal256,
    /// Total ASTRO lockdrop incentives to be distributed among the users
    pub lockdrop_incentives: Uint256,
}




#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    /// ASTRO Tokens delegated to the bootstrap auction contract
    pub total_astro_delegated: Uint256,
    /// ASTRO returned to forcefully unlock Lockup positions
    pub total_astro_returned: Uint256,
    /// Boolean value indicating if the user can withdraw thier ASTRO rewards or not
    pub are_claims_allowed: bool,
    /// Vector containing LP addresses for all the supported LP Pools
    pub supported_lp_tokens: Vec<String>,
}






#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PoolInfo {
    /// LP Token Address
    pub lp_token_addr: Addr,
    /// Pool Address
    pub pool_addr: Addr,
    /// Dual Reward Address
    pub dual_reward_addr: Addr,
    /// % of total ASTRO incentives allocated to this pool
    pub incentives_percent: Decimal256,
    /// LP Token balance before liquidity migration to astroport 
    pub total_lp_units_before_migration: Uint256,
    /// Astroport LP Token balance post liquidity migration to astroport 
    pub total_lp_units_after_migration: Uint256,
    /// Boolean value indicating if the LP Tokens are staked with the Generator contract or not
    pub is_staked: bool,
    /// Pool Type :: Astro or terra ? 
    pub pool_type: String,
    /// Boolean value indicating if the liquidity has been migrated or not 
    pub is_migrated: bool,
    /// Weighted LP Token balance used to calculate ASTRO rewards a particular user can claim
    pub weighted_amount: Uint256,
    /// Ratio of ASTRO rewards accured to total_lp_deposited. Used to calculate ASTRO incentives accured by each user
    pub astro_global_reward_index: Decimal256,
    /// Ratio of ASSET rewards accured to total_lp_deposited. Used to calculate ASSET incentives accured by each user
    pub asset_global_reward_index: Decimal256,
}


impl Default for PoolInfo {
    fn default() -> Self {
        PoolInfo {
            lp_token_addr: zero_address(),
            pool_addr: zero_address(),
            dual_reward_addr: zero_address(),
            incentives_percent: Decimal256::zero(),
            total_lp_units_before_migration: Uint256::zero(),
            total_lp_units_after_migration: Uint256::zero(),
            is_staked: false,
            pool_type: "terraswap".to_string(),
            is_migrated: false,
            weighted_amount: Uint256::zero(),
            astro_global_reward_index: Decimal256::zero(),
            asset_global_reward_index: Decimal256::zero()
        }
    }
}




#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UserInfo {
    /// Total ASTRO tokens user received as rewards for participation in the lockdrop
    pub total_astro_rewards: Uint256,
    /// Total ASTRO tokens user can still withdraw 
    pub unclaimed_astro_rewards: Uint256,
    /// Total ASTRO tokens user delegated to the LP bootstrap auction pool 
    pub delegated_astro_rewards: Uint256,
    /// Contains lockup Ids of the User's lockup positions with different pools having different durations / deposit amounts
    pub lockup_positions: Vec<String>,
}


impl Default for UserInfo {
    fn default() -> Self {
        UserInfo {
            total_astro_rewards: Uint256::zero(),
            unclaimed_astro_rewards: Uint256::zero(),
            delegated_astro_rewards: Uint256::zero(),
            lockup_positions: vec![]
        }
    }
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LockupInfo {
    /// LP Pool identifer whose LP tokens this Lockdrop position accounts for 
    pub pool_lp_token_addr: Addr,
    /// Lockup Duration
    pub duration: u64,
    /// UST locked as part of this lockup position
    pub lp_units_locked: Uint256,
    /// UST locked as part of this lockup position
    pub astro_rewards: Uint256,
    /// Boolean value indicating if the user's LP units have been updated post liquidity migration
    pub is_migrated: bool,
    /// Boolean value indicating if the user's has withdrawn funds post the only 1 withdrawal limit cutoff
    pub withdrawal_counter: bool,
    /// Timestamp beyond which this position can be unlocked
    pub unlock_timestamp: u64,
    /// Used to calculate user's pending ASTRO rewards from the generator (staking) contract 
    pub astro_reward_index: Decimal256,
    /// Used to calculate user's pending DUAL rewards from the generator (staking) contract 
    pub dual_reward_index: Decimal256,
}


impl Default for LockupInfo {
    fn default() -> Self {
        LockupInfo {
            pool_lp_token_addr: Addr::unchecked(""),
            duration: 0 as u64,
            lp_units_locked: Uint256::zero(),
            astro_rewards: Uint256::zero(),
            is_migrated: false,
            withdrawal_counter: false,
            unlock_timestamp: 0 as u64,
            astro_reward_index: Decimal256::zero(),
            dual_reward_index: Decimal256::zero(),
        }
    }
}




