use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, Decimal, HumanAddr, ReadonlyStorage, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

use crate::msg::{DelegateResponse, TokenInfoResponse};

pub const KEY_DELEGATORS: &[u8] = b"delegator";
pub const KEY_INVESTMENT: &[u8] = b"invest";
pub const KEY_TOKEN_INFO: &[u8] = b"token";
pub const KEY_TOTAL_SUPPLY: &[u8] = b"total_supply";

pub const PREFIX_BALANCE: &[u8] = b"balance";
pub const PREFIX_CLAIMS: &[u8] = b"claim";
pub const PREFIX_DELEGATIONS: &[u8] = b"delegation";

/// balances are state of the erc20 tokens
pub fn balances<S: Storage>(storage: &mut S) -> Bucket<S, Uint128> {
    bucket(storage, PREFIX_BALANCE)
}

pub fn balances_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyBucket<S, Uint128> {
    bucket_read(storage, PREFIX_BALANCE)
}

/// claims are the claims to money being unbonded
pub fn claims<S: Storage>(storage: &mut S) -> Bucket<S, Uint128> {
    bucket(storage, PREFIX_CLAIMS)
}

pub fn claims_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyBucket<S, Uint128> {
    bucket_read(storage, PREFIX_CLAIMS)
}

pub fn delegations<S: Storage>(storage: &mut S) -> Bucket<S, DelegateInfo> {
    bucket(storage, PREFIX_DELEGATIONS)
}

pub fn delegations_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyBucket<S, DelegateResponse> {
    bucket_read(storage, PREFIX_DELEGATIONS)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct DelegateInfo {
    pub delegator: HumanAddr,
    pub validator: HumanAddr,
    pub amount: Uint128,
    pub last_delegate_height: u64,
    pub unbond_flag: bool,
    pub undelegate_reward: Uint128,
}

pub fn delegators<S: Storage>(storage: &mut S) -> Singleton<S, Vec<HumanAddr>> {
    singleton(storage, KEY_DELEGATORS)
}

pub fn delegators_read<S: ReadonlyStorage>(storage: &S) -> ReadonlySingleton<S, Vec<HumanAddr>> {
    singleton_read(storage, KEY_DELEGATORS)
}

/// Investment info is fixed at initialization, and is used to control the function of the contract
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InvestmentInfo {
    /// owner created the contract and takes a cut
    pub owner: CanonicalAddr,
    /// this is the denomination we can stake (and only one we accept for payments)
    pub bond_denom: String,
    /// this is how much the owner takes as a cut when someone unbonds
    pub exit_tax: Decimal,
    /// All tokens are bonded to this validator
    /// FIXME: humanize/canonicalize address doesn't work for validator addrresses
    pub validator: HumanAddr,
    /// This is the minimum amount we will pull out to reinvest, as well as a minumum
    /// that can be unbonded (to avoid needless staking tx)
    pub min_withdrawal: Uint128,
}

/// Supply is dynamic and tracks the current supply of staked and ERC20 tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct Supply {
    /// issued is how many derivative tokens this contract has issued
    pub issued: Uint128,
    /// bonded is how many native tokens exist bonded to the validator
    pub bonded: Uint128,
    /// claims is how many tokens need to be reserved paying back those who unbonded
    pub claims: Uint128,
}

pub fn invest_info<S: Storage>(storage: &mut S) -> Singleton<S, InvestmentInfo> {
    singleton(storage, KEY_INVESTMENT)
}

pub fn invest_info_read<S: ReadonlyStorage>(storage: &S) -> ReadonlySingleton<S, InvestmentInfo> {
    singleton_read(storage, KEY_INVESTMENT)
}

pub fn token_info<S: Storage>(storage: &mut S) -> Singleton<S, TokenInfoResponse> {
    singleton(storage, KEY_TOKEN_INFO)
}

pub fn token_info_read<S: ReadonlyStorage>(storage: &S) -> ReadonlySingleton<S, TokenInfoResponse> {
    singleton_read(storage, KEY_TOKEN_INFO)
}

pub fn total_supply<S: Storage>(storage: &mut S) -> Singleton<S, Supply> {
    singleton(storage, KEY_TOTAL_SUPPLY)
}

pub fn total_supply_read<S: ReadonlyStorage>(storage: &S) -> ReadonlySingleton<S, Supply> {
    singleton_read(storage, KEY_TOTAL_SUPPLY)
}
