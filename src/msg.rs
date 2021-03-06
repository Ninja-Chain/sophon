use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Coin, Decimal, HumanAddr, Uint128};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    /// name of the derivative token (FIXME: auto-generate?)
    pub name: String,
    /// symbol / ticker of the derivative token
    pub symbol: String,
    /// decimal places of the derivative token (for UI)
    /// TODO: does this make sense? Do we need to normalize on this?
    /// We don't even know the decimals of the native token
    pub decimals: u8,

    /// This is the validator that all tokens will be bonded to
    pub validator: HumanAddr,

    /// this is how much the owner takes as a cut when someone unbonds
    /// TODO
    pub exit_tax: Decimal,
    /// This is the minimum amount we will pull out to reinvest, as well as a minumum
    /// that can be unbonded (to avoid needless staking tx)
    pub min_withdrawal: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Transfer moves the derivative token
    Transfer {
        recipient: HumanAddr,
        amount: Uint128,
    },
    /// Bond will bond all staking tokens sent with the message and release derivative tokens
    Bond {},
    /// Unbond will "burn" the given amount of derivative tokens and send the unbonded
    /// staking tokens to the message sender (after exit tax is deducted)
    Unbond {},
    /// _BondAllTokens can only be called by the contract itself, after all rewards have been
    /// withdrawn. This is an example of using "callbacks" in message flows.
    /// This can only be invoked by the contract itself as a return from Reinvest
    _BondAllTokens {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Balance shows the number of staking derivatives
    Balance { address: HumanAddr },
    /// Claims shows the number of tokens this address can access when they are done unbonding
    Claims { address: HumanAddr },
    /// TokenInfo shows the metadata of the token for UIs
    TokenInfo {},
    /// Investment shows info on total staking tokens under custody,
    /// with which validator, as well as how many derivative tokens are lists.
    /// It also shows with the exit tax.
    Investment {},
    Validators {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct BalanceResponse {
    pub balance: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ClaimsResponse {
    pub claims: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DelegateResponse{
    pub delegator: HumanAddr,
    pub validator: HumanAddr,
    pub amount: Uint128,
    pub last_delegate_height: u64,
    pub unbond_flag: bool,
    pub undelegate_reward: Uint128,
}

/// TokenInfoResponse is info to display the derivative token in a UI
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct TokenInfoResponse {
    /// name of the derivative token
    pub name: String,
    /// symbol / ticker of the derivative token
    pub symbol: String,
    /// decimal places of the derivative token (for UI)
    pub decimals: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InvestmentResponse {
    pub token_supply: Uint128,
    pub staked_tokens: Coin,
    // ratio of staked_tokens / token_supply (or how many native tokens that one derivative token is nominally worth)
    pub nominal_value: Decimal,

    /// owner created the contract and takes a cut
    pub owner: HumanAddr,
    /// this is how much the owner takes as a cut when someone unbonds
    pub exit_tax: Decimal,
    /// All tokens are bonded to this validator
    pub validator: HumanAddr,
    /// This is the minimum amount we will pull out to reinvest, as well as a minumum
    /// that can be unbonded (to avoid needless staking tx)
    pub min_withdrawal: Uint128,
}
