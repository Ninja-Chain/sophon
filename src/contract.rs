use cosmwasm_std::{
    attr, coin, Coin, to_binary, Api, CosmosMsg, BankMsg, Binary, Decimal, Env, Extern, HandleResponse, HumanAddr,
    InitResponse, MessageInfo, Querier, QueryRequest, StakingMsg, StakingQuery, StdError,
    StdResult, Storage, Uint128, Validator, ValidatorsResponse
};

use crate::errors::{StakingError, Unauthorized};
use crate::msg::{
    BalanceResponse, ClaimsResponse, DelegateResponse, HandleMsg, InitMsg, InvestmentResponse,
    QueryMsg, TokenInfoResponse,
};
use crate::state::{
    balances, balances_read, claims_read, delegations, delegations_read, delegators,
    delegators_read, invest_info, invest_info_read, token_info, token_info_read, total_supply,
    total_supply_read, InvestmentInfo, Supply,
};

const FALLBACK_RATIO: Decimal = Decimal::one();

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    info: MessageInfo,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // ensure the validator is registered
    let vals = deps.querier.query_validators()?;
    if !vals.iter().any(|v| v.address == msg.validator) {
        return Err(StdError::generic_err(format!(
            "{} is not in the current validator set",
            msg.validator
        )));
    }

    let token = TokenInfoResponse {
        name: msg.name,
        symbol: msg.symbol,
        decimals: msg.decimals,
    };
    token_info(&mut deps.storage).save(&token)?;

    let denom = deps.querier.query_bonded_denom()?;

    let invest = InvestmentInfo {
        owner: deps.api.canonical_address(&info.sender)?,
        exit_tax: msg.exit_tax,
        bond_denom: denom,
        validator: msg.validator,
        min_withdrawal: msg.min_withdrawal,
    };
    invest_info(&mut deps.storage).save(&invest)?;

    // set supply to 0
    let supply = Supply::default();
    total_supply(&mut deps.storage).save(&supply)?;

    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
    msg: HandleMsg,
) -> Result<HandleResponse, StakingError> {
    match msg {
        HandleMsg::Transfer { recipient, amount } => {
            Ok(transfer(deps, env, info, recipient, amount)?)
        }
        HandleMsg::Bond {} => Ok(bond(deps, env, info)?),
        HandleMsg::Unbond {} => Ok(reserve_unbond(deps, env, info)?),
        HandleMsg::_BondAllTokens {} => _bond_all_tokens(deps, env, info),
    }
}

pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    info: MessageInfo,
    recipient: HumanAddr,
    send: Uint128,
) -> StdResult<HandleResponse> {
    let rcpt_raw = deps.api.canonical_address(&recipient)?;
    let sender_raw = deps.api.canonical_address(&info.sender)?;

    let mut accounts = balances(&mut deps.storage);
    accounts.update(&sender_raw, |balance: Option<Uint128>| {
        balance.unwrap_or_default() - send
    })?;
    accounts.update(&rcpt_raw, |balance: Option<Uint128>| -> StdResult<_> {
        Ok(balance.unwrap_or_default() + send)
    })?;

    let res = HandleResponse {
        messages: vec![],
        attributes: vec![
            attr("action", "transfer"),
            attr("from", info.sender),
            attr("to", recipient),
            attr("amount", send),
        ],
        data: None,
    };
    Ok(res)
}

// get_bonded returns the total amount of delegations from contract
// it ensures they are all the same denom
fn get_bonded<Q: Querier>(querier: &Q, contract: &HumanAddr) -> StdResult<Uint128> {
    let bonds = querier.query_all_delegations(contract)?;
    if bonds.is_empty() {
        return Ok(Uint128(0));
    }
    let denom = bonds[0].amount.denom.as_str();
    bonds.iter().fold(Ok(Uint128(0)), |racc, d| {
        let acc = racc?;
        if d.amount.denom.as_str() != denom {
            Err(StdError::generic_err(format!(
                "different denoms in bonds: '{}' vs '{}'",
                denom, &d.amount.denom
            )))
        } else {
            Ok(acc + d.amount.amount)
        }
    })
}

fn assert_bonds(supply: &Supply, bonded: Uint128) -> StdResult<()> {
    if supply.bonded != bonded {
        Err(StdError::generic_err(format!(
            "Stored bonded {}, but query bonded: {}",
            supply.bonded, bonded
        )))
    } else {
        Ok(())
    }
}

pub fn bond<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
) -> StdResult<HandleResponse> {
    let delegator_raw = deps.api.canonical_address(&info.sender)?;
    let best_validator = select_validator(deps)?;

    let invest = invest_info_read(&deps.storage).load()?;
    let info_clone = info.clone();
    let payment = info_clone
        .sent_funds
        .iter()
        .find(|x| x.denom == invest.bond_denom)
        .ok_or_else(|| StdError::generic_err(format!("No {} tokens sent", &invest.bond_denom)))?;

    delegations(&mut deps.storage).update(
        delegator_raw.as_slice(),
        |delegate_info| -> StdResult<_> {
            let mut new_delegate_info = delegate_info.unwrap();
            new_delegate_info.undelegate_reward = Uint128::zero();
            new_delegate_info.amount = payment.clone().amount;
            new_delegate_info.validator = best_validator.address.clone();
            new_delegate_info.last_delegate_height = env.clone().block.height;
            Ok(new_delegate_info)
        },
    )?;

    is_expired(deps, env, info.clone());

    let attributes = vec![
        attr("action", "bond"),
        attr("from", info.sender),
        attr("validator", best_validator.address.clone()),
        attr("bonded", payment.clone().amount),
    ];

    let r = HandleResponse {
        messages: vec![StakingMsg::Delegate {
            validator: best_validator.address.clone(),
            amount: payment.clone(),
        }
        .into()],
        attributes,
        data: None,
    };

    Ok(r)
}

pub fn reserve_unbond<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
) -> StdResult<HandleResponse> {
    let delegator = info.clone().sender;
    claim(deps, env.clone(), delegator);

    let delegator_raw = deps.api.canonical_address(&info.sender)?;
    delegations(&mut deps.storage).update(
        delegator_raw.as_slice(),
        |delegate_info| -> StdResult<_> {
            let mut new_delegate_info = delegate_info.unwrap();
            new_delegate_info.unbond_flag = true;
            Ok(new_delegate_info)
        },
    )?;

    return is_expired(deps, env, info);
}

fn claim<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    delegator: HumanAddr,
) -> Result<HandleResponse, StakingError> {
    let validator_addr = query_delegation(deps, delegator).unwrap().validator;
    let all_delegations = query_all_delegations(deps).unwrap();
    let delegations_of_val = all_delegations
        .iter()
        .filter(|delegation| delegation.validator == validator_addr);

    let mut total_amount = Uint128::zero();
    for delegation in delegations_of_val.clone() {
        total_amount += delegation.amount
    }

    // find how many tokens we have to bond
    let invest = invest_info_read(&deps.storage).load()?;
    let balance = deps
        .querier
        .query_balance(&env.contract.address, &invest.bond_denom)?;

    let reward = (balance.amount - total_amount).unwrap();

    for delegation in delegations_of_val {
        let key = deps.api.canonical_address(&delegation.delegator)?;
        delegations(&mut deps.storage).update(key.as_slice(), |delegate_info| -> StdResult<_> {
            let mut new_delegate_info = delegate_info.unwrap();
            new_delegate_info.undelegate_reward = reward
                .clone()
                .multiply_ratio(delegation.amount, total_amount);
            Ok(new_delegate_info)
        })?;
    }

    Ok(HandleResponse {
        messages: vec![],
        attributes: vec![],
        data: None,
    })
}

/// reinvest will withdraw all pending rewards,
/// then issue a callback to itself via _bond_all_tokens
/// to reinvest the new earnings (and anything else that accumulated)
fn reinvest<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
    delegator: HumanAddr,
) -> StdResult<HandleResponse> {
    let _ = claim(deps, env.clone(), delegator.clone());

    let best_validator = select_validator(deps)?;

    let delegator_raw = deps.api.canonical_address(&delegator)?;
    let delegate_info = delegations_read(&deps.storage)
        .may_load(delegator_raw.as_slice())
        .unwrap_or_default()
        .unwrap();
    let prev_validator = delegate_info.validator;
    let undelegated_amount = delegate_info.undelegate_reward;
    let delegated_amount = delegate_info.amount;

    delegations(&mut deps.storage).update(
        delegator_raw.as_slice(),
        |delegate_info| -> StdResult<_> {
            let mut new_delegate_info = delegate_info.unwrap();
            new_delegate_info.undelegate_reward = Uint128::zero();
            new_delegate_info.amount += undelegated_amount;
            new_delegate_info.validator = best_validator.address.clone();
            new_delegate_info.last_delegate_height = env.block.height;
            Ok(new_delegate_info)
        },
    )?;

    let token_info_res = query_token_info(deps)?;

    let attributes = vec![
        attr("action", "reinvest"),
        attr("prev_validator", prev_validator.clone()),
        attr("new_validator", best_validator.address.clone()),
        attr(
            "amount",
            undelegated_amount.clone() + delegated_amount.clone(),
        ),
    ];

    let r = HandleResponse {
        messages: vec![
            StakingMsg::Delegate {
                amount: coin(undelegated_amount.u128(), token_info_res.name.clone()),
                validator: best_validator.address.clone(),
            }
            .into(),
            StakingMsg::Redelegate {
                amount: coin(delegated_amount.u128(), token_info_res.name),
                dst_validator: best_validator.address,
                src_validator: prev_validator,
            }
            .into(),
        ],
        attributes,
        data: None,
    };

    Ok(r)
}

pub fn _bond_all_tokens<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
) -> Result<HandleResponse, StakingError> {
    // this is just meant as a call-back to ourself
    if info.sender != env.contract.address {
        return Err(Unauthorized {}.build());
    }

    // find how many tokens we have to bond
    let invest = invest_info_read(&deps.storage).load()?;
    let mut balance = deps
        .querier
        .query_balance(&env.contract.address, &invest.bond_denom)?;

    // we deduct pending claims from our account balance before reinvesting.
    // if there is not enough funds, we just return a no-op
    match total_supply(&mut deps.storage).update(|mut supply| {
        balance.amount = (balance.amount - supply.claims)?;
        // this just triggers the "no op" case if we don't have min_withdrawal left to reinvest
        (balance.amount - invest.min_withdrawal)?;
        supply.bonded += balance.amount;
        Ok(supply)
    }) {
        Ok(_) => {}
        // if it is below the minimum, we do a no-op (do not revert other state from withdrawal)
        Err(StdError::Underflow { .. }) => return Ok(HandleResponse::default()),
        Err(e) => return Err(e.into()),
    }

    // and bond them to the validator
    let res = HandleResponse {
        messages: vec![StakingMsg::Delegate {
            validator: invest.validator,
            amount: balance.clone(),
        }
        .into()],
        attributes: vec![attr("action", "reinvest"), attr("bonded", balance.amount)],
        data: None,
    };
    Ok(res)
}

fn select_validator<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
) -> StdResult<Validator> {
    let validators = deps.querier.query_validators()?;
    let min_commission = validators
        .iter()
        .min_by_key(|v| v.commission)
        .unwrap()
        .commission;
    let validator = validators
        .iter()
        .filter(|v| v.commission == min_commission)
        .min_by_key(|v| v.max_change_rate)
        .unwrap();
    Ok(validator.clone())
}

fn unbond<S: Storage, A: Api, Q: Querier> (
    deps: &mut Extern<S, A, Q>,
    env: Env,
    delegator: HumanAddr,
) -> StdResult<HandleResponse> {
    // アドレスに対応するDelegateInfoのamountとundelegate_rewardをクエリする
    let delegation = query_delegation(deps,delegator.clone())?;
    let amount = delegation.amount;
    let undelegate_reward = delegation.undelegate_reward;

    // アドレスに対応するDelegateInfoのunbond_flagをfalseに、amountを0に更新する
    let key = deps.api.canonical_address(&delegation.delegator)?;
    delegations(&mut deps.storage).update(key.as_slice(), |delegate_info| -> StdResult<_> {
        let mut new_delegate_info = delegate_info.unwrap();
        new_delegate_info.unbond_flag = false;
        new_delegate_info.amount = Uint128::zero();
        new_delegate_info.undelegate_reward = Uint128::zero();

        Ok(new_delegate_info)
    })?;

    let unbound_amount = vec![Coin::new((amount + undelegate_reward).u128(), "stake")];

    // 引数のアドレスに対して、amountの量のstakeを送金する
    send_tokens(
        env.contract.address,
        delegator,
        unbound_amount,
        "approve",
    )
}

fn send_tokens(
    from_address: HumanAddr,
    to_address: HumanAddr,
    amount: Vec<Coin>,
    action: &str,
) -> StdResult<HandleResponse> {
    let attributes = vec![attr("action", action), attr("to", to_address.clone())];

    let r = HandleResponse {
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address,
            to_address,
            amount,
        })],
        attributes,
        data: None,
    };
    Ok(r)
}

fn is_expired<S: Storage, A: Api, Q: Querier> (
    deps: &mut Extern<S, A, Q>,
    env: Env,
    info: MessageInfo,
) -> StdResult<HandleResponse> {
    let delegator_list = query_all_delegators(deps).unwrap();
    let block_height = env.block.height;
    for address in delegator_list.into_iter() {
        let delegation = query_delegation(deps, address.clone()).unwrap();
        if delegation.last_delegate_height - block_height > 25920 {
            if delegation.unbond_flag == true {
                unbond(deps, env.clone(), address);
            } else {
                reinvest(deps, env.clone(), info.clone(), address);
            };
        };
    };

    Ok(HandleResponse {
        messages: vec![],
        attributes: vec![],
        data: None,
    })

}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    _env: Env,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::TokenInfo {} => to_binary(&query_token_info(deps)?),
        QueryMsg::Investment {} => to_binary(&query_investment(deps)?),
        QueryMsg::Balance { address } => to_binary(&query_balance(deps, address)?),
        QueryMsg::Claims { address } => to_binary(&query_claims(deps, address)?),
        QueryMsg::Validators {} => to_binary(&query_validators(deps)?),
    }
}

fn query_all_delegations<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<Vec<DelegateResponse>> {
    let delegator_list = query_all_delegators(deps).unwrap();
    let mut delegations = vec![];
    for address in delegator_list.into_iter() {
        let delegation = query_delegation(deps, address);
        delegations.append(&mut vec![delegation.unwrap()])
    }
    Ok(delegations)
}

fn query_delegation<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<DelegateResponse> {
    let address_raw = deps.api.canonical_address(&address)?;
    let delegation = delegations_read(&deps.storage)
        .may_load(address_raw.as_slice())
        .unwrap_or_default();
    Ok(delegation.unwrap())
}

fn query_all_delegators<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<Vec<HumanAddr>> {
    delegators_read(&deps.storage).load()
}

pub fn query_token_info<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<TokenInfoResponse> {
    token_info_read(&deps.storage).load()
}

pub fn query_balance<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<BalanceResponse> {
    let address_raw = deps.api.canonical_address(&address)?;
    let balance = balances_read(&deps.storage)
        .may_load(address_raw.as_slice())?
        .unwrap_or_default();
    Ok(BalanceResponse { balance })
}

pub fn query_claims<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<ClaimsResponse> {
    let address_raw = deps.api.canonical_address(&address)?;
    let claims = claims_read(&deps.storage)
        .may_load(address_raw.as_slice())?
        .unwrap_or_default();
    Ok(ClaimsResponse { claims })
}

pub fn query_investment<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<InvestmentResponse> {
    let invest = invest_info_read(&deps.storage).load()?;
    let supply = total_supply_read(&deps.storage).load()?;

    let res = InvestmentResponse {
        owner: deps.api.human_address(&invest.owner)?,
        exit_tax: invest.exit_tax,
        validator: invest.validator,
        min_withdrawal: invest.min_withdrawal,
        token_supply: supply.issued,
        staked_tokens: coin(supply.bonded.u128(), &invest.bond_denom),
        nominal_value: if supply.issued.is_zero() {
            FALLBACK_RATIO
        } else {
            Decimal::from_ratio(supply.bonded, supply.issued)
        },
    };
    Ok(res)
}

fn query_validators<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<Vec<Validator>> {
    let request = StakingQuery::Validators {}.into();
    let res: ValidatorsResponse = deps.querier.query(&QueryRequest::Staking(request))?;
    Ok(res.validators)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{
        mock_dependencies, mock_env, mock_info, MockQuerier, MOCK_CONTRACT_ADDR,
    };
    use cosmwasm_std::{coins, Coin, CosmosMsg, Decimal, FullDelegation, Validator};
    use std::str::FromStr;

    fn sample_validator<U: Into<HumanAddr>>(addr: U) -> Validator {
        Validator {
            address: addr.into(),
            commission: Decimal::percent(3),
            max_commission: Decimal::percent(10),
            max_change_rate: Decimal::percent(1),
        }
    }

    fn custom_sample_validator<U: Into<HumanAddr>>(
        addr: U,
        comm: u64,
        max_comm: u64,
        rate: u64,
    ) -> Validator {
        Validator {
            address: addr.into(),
            commission: Decimal::percent(comm),
            max_commission: Decimal::percent(max_comm),
            max_change_rate: Decimal::percent(rate),
        }
    }

    fn sample_delegation<U: Into<HumanAddr>>(addr: U, amount: Coin) -> FullDelegation {
        let can_redelegate = amount.clone();
        FullDelegation {
            validator: addr.into(),
            delegator: HumanAddr::from(MOCK_CONTRACT_ADDR),
            amount,
            can_redelegate,
            accumulated_rewards: Vec::new(),
        }
    }

    fn set_validator(querier: &mut MockQuerier) {
        querier.update_staking("ustake", &[sample_validator(DEFAULT_VALIDATOR)], &[]);
    }

    fn set_delegation(querier: &mut MockQuerier, amount: u128, denom: &str) {
        querier.update_staking(
            "ustake",
            &[sample_validator(DEFAULT_VALIDATOR)],
            &[sample_delegation(DEFAULT_VALIDATOR, coin(amount, denom))],
        );
    }

    const DEFAULT_VALIDATOR: &str = "default-validator";

    fn default_init(tax_percent: u64, min_withdrawal: u128) -> InitMsg {
        InitMsg {
            name: "Cool Derivative".to_string(),
            symbol: "DRV".to_string(),
            decimals: 9,
            validator: HumanAddr::from(DEFAULT_VALIDATOR),
            exit_tax: Decimal::percent(tax_percent),
            min_withdrawal: Uint128(min_withdrawal),
        }
    }

    fn get_balance<S: Storage, A: Api, Q: Querier, U: Into<HumanAddr>>(
        deps: &Extern<S, A, Q>,
        addr: U,
    ) -> Uint128 {
        query_balance(&deps, addr.into()).unwrap().balance
    }

    fn get_claims<S: Storage, A: Api, Q: Querier, U: Into<HumanAddr>>(
        deps: &Extern<S, A, Q>,
        addr: U,
    ) -> Uint128 {
        query_claims(&deps, addr.into()).unwrap().claims
    }

    #[test]
    fn initialization_with_missing_validator() {
        let mut deps = mock_dependencies(&[]);
        deps.querier
            .update_staking("ustake", &[sample_validator("john")], &[]);

        let creator = HumanAddr::from("creator");
        let msg = InitMsg {
            name: "Cool Derivative".to_string(),
            symbol: "DRV".to_string(),
            decimals: 9,
            validator: HumanAddr::from("my-validator"),
            exit_tax: Decimal::percent(2),
            min_withdrawal: Uint128(50),
        };
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, msg.clone());
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => {
                assert_eq!(msg, "my-validator is not in the current validator set")
            }
            _ => panic!("expected unregistered validator error"),
        }
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(&[]);
        deps.querier.update_staking(
            "ustake",
            &[
                sample_validator("john"),
                sample_validator("mary"),
                sample_validator("my-validator"),
            ],
            &[],
        );

        let creator = HumanAddr::from("creator");
        let msg = InitMsg {
            name: "Cool Derivative".to_string(),
            symbol: "DRV".to_string(),
            decimals: 0,
            validator: HumanAddr::from("my-validator"),
            exit_tax: Decimal::percent(2),
            min_withdrawal: Uint128(50),
        };
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, msg.clone()).unwrap();
        assert_eq!(0, res.messages.len());

        // token info is proper
        let token = query_token_info(&deps).unwrap();
        assert_eq!(&token.name, &msg.name);
        assert_eq!(&token.symbol, &msg.symbol);
        assert_eq!(token.decimals, msg.decimals);

        // no balance
        assert_eq!(get_balance(&deps, &creator), Uint128(0));
        // no claims
        assert_eq!(get_claims(&deps, &creator), Uint128(0));

        // investment info correct
        let invest = query_investment(&deps).unwrap();
        assert_eq!(&invest.owner, &creator);
        assert_eq!(&invest.validator, &msg.validator);
        assert_eq!(invest.exit_tax, msg.exit_tax);
        assert_eq!(invest.min_withdrawal, msg.min_withdrawal);

        assert_eq!(invest.token_supply, Uint128(0));
        assert_eq!(invest.staked_tokens, coin(0, "ustake"));
        assert_eq!(invest.nominal_value, Decimal::one());
    }

    #[test]
    fn bonding_issues_tokens() {
        let mut deps = mock_dependencies(&[]);
        set_validator(&mut deps.querier);

        let creator = HumanAddr::from("creator");
        let init_msg = default_init(2, 50);
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        // let's bond some tokens now
        let bob = HumanAddr::from("bob");
        let bond_msg = HandleMsg::Bond {};
        let info = mock_info(&bob, &[coin(10, "random"), coin(1000, "ustake")]);

        // try to bond and make sure we trigger delegation
        let res = handle(&mut deps, mock_env(), info, bond_msg).unwrap();
        assert_eq!(1, res.messages.len());
        let delegate = &res.messages[0];
        match delegate {
            CosmosMsg::Staking(StakingMsg::Delegate { validator, amount }) => {
                assert_eq!(validator.as_str(), DEFAULT_VALIDATOR);
                assert_eq!(amount, &coin(1000, "ustake"));
            }
            _ => panic!("Unexpected message: {:?}", delegate),
        }

        // bob got 1000 DRV for 1000 stake at a 1.0 ratio
        assert_eq!(get_balance(&deps, &bob), Uint128(1000));

        // investment info correct (updated supply)
        let invest = query_investment(&deps).unwrap();
        assert_eq!(invest.token_supply, Uint128(1000));
        assert_eq!(invest.staked_tokens, coin(1000, "ustake"));
        assert_eq!(invest.nominal_value, Decimal::one());
    }

    #[test]
    fn rebonding_changes_pricing() {
        let mut deps = mock_dependencies(&[]);
        set_validator(&mut deps.querier);

        let creator = HumanAddr::from("creator");
        let init_msg = default_init(2, 50);
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        // let's bond some tokens now
        let bob = HumanAddr::from("bob");
        let bond_msg = HandleMsg::Bond {};
        let info = mock_info(&bob, &[coin(10, "random"), coin(1000, "ustake")]);
        let res = handle(&mut deps, mock_env(), info, bond_msg).unwrap();
        assert_eq!(1, res.messages.len());

        // update the querier with new bond
        set_delegation(&mut deps.querier, 1000, "ustake");

        // fake a reinvestment (this must be sent by the contract itself)
        let rebond_msg = HandleMsg::_BondAllTokens {};
        let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
        deps.querier
            .update_balance(MOCK_CONTRACT_ADDR, coins(500, "ustake"));
        let _ = handle(&mut deps, mock_env(), info, rebond_msg).unwrap();

        // update the querier with new bond
        set_delegation(&mut deps.querier, 1500, "ustake");

        // we should now see 1000 issues and 1500 bonded (and a price of 1.5)
        let invest = query_investment(&deps).unwrap();
        assert_eq!(invest.token_supply, Uint128(1000));
        assert_eq!(invest.staked_tokens, coin(1500, "ustake"));
        let ratio = Decimal::from_str("1.5").unwrap();
        assert_eq!(invest.nominal_value, ratio);

        // we bond some other tokens and get a different issuance price (maintaining the ratio)
        let alice = HumanAddr::from("alice");
        let bond_msg = HandleMsg::Bond {};
        let info = mock_info(&alice, &[coin(3000, "ustake")]);
        let res = handle(&mut deps, mock_env(), info, bond_msg).unwrap();
        assert_eq!(1, res.messages.len());

        // update the querier with new bond
        set_delegation(&mut deps.querier, 3000, "ustake");

        // alice should have gotten 2000 DRV for the 3000 stake, keeping the ratio at 1.5
        assert_eq!(get_balance(&deps, &alice), Uint128(2000));

        let invest = query_investment(&deps).unwrap();
        assert_eq!(invest.token_supply, Uint128(3000));
        assert_eq!(invest.staked_tokens, coin(4500, "ustake"));
        assert_eq!(invest.nominal_value, ratio);
    }

    #[test]
    fn bonding_fails_with_wrong_denom() {
        let mut deps = mock_dependencies(&[]);
        set_validator(&mut deps.querier);

        let creator = HumanAddr::from("creator");
        let init_msg = default_init(2, 50);
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        // let's bond some tokens now
        let bob = HumanAddr::from("bob");
        let bond_msg = HandleMsg::Bond {};
        let info = mock_info(&bob, &[coin(500, "photon")]);

        // try to bond and make sure we trigger delegation
        let res = handle(&mut deps, mock_env(), info, bond_msg);
        match res.unwrap_err() {
            StakingError::Std {
                original: StdError::GenericErr { msg, .. },
            } => assert_eq!(msg, "No ustake tokens sent"),
            err => panic!("Unexpected error: {:?}", err),
        };
    }

    #[test]
    fn unbonding_maintains_price_ratio() {
        let mut deps = mock_dependencies(&[]);
        set_validator(&mut deps.querier);

        let creator = HumanAddr::from("creator");
        let init_msg = default_init(10, 50);
        let info = mock_info(&creator, &[]);

        // make sure we can init with this
        let res = init(&mut deps, mock_env(), info, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        // let's bond some tokens now
        let bob = HumanAddr::from("bob");
        let bond_msg = HandleMsg::Bond {};
        let info = mock_info(&bob, &[coin(10, "random"), coin(1000, "ustake")]);
        let res = handle(&mut deps, mock_env(), info, bond_msg).unwrap();
        assert_eq!(1, res.messages.len());

        // update the querier with new bond
        set_delegation(&mut deps.querier, 1000, "ustake");

        // fake a reinvestment (this must be sent by the contract itself)
        // after this, we see 1000 issues and 1500 bonded (and a price of 1.5)
        let rebond_msg = HandleMsg::_BondAllTokens {};
        let info = mock_info(MOCK_CONTRACT_ADDR, &[]);
        deps.querier
            .update_balance(MOCK_CONTRACT_ADDR, coins(500, "ustake"));
        let _ = handle(&mut deps, mock_env(), info, rebond_msg).unwrap();

        // update the querier with new bond, lower balance
        set_delegation(&mut deps.querier, 1500, "ustake");
        deps.querier.update_balance(MOCK_CONTRACT_ADDR, vec![]);

        // creator now tries to unbond these tokens - this must fail
        let unbond_msg = HandleMsg::Unbond {
            amount: Uint128(600),
        };
        let info = mock_info(&creator, &[]);
        let res = handle(&mut deps, mock_env(), info, unbond_msg);
        match res.unwrap_err() {
            StakingError::Std {
                original: StdError::Underflow { .. },
            } => {}
            err => panic!("Unexpected error: {:?}", err),
        }

        // bob unbonds 600 tokens at 10% tax...
        // 60 are taken and send to the owner
        // 540 are unbonded in exchange for 540 * 1.5 = 810 native tokens
        let unbond_msg = HandleMsg::Unbond {
            amount: Uint128(600),
        };
        let owner_cut = Uint128(60);
        let bobs_claim = Uint128(810);
        let bobs_balance = Uint128(400);
        let info = mock_info(&bob, &[]);
        let res = handle(&mut deps, mock_env(), info, unbond_msg).unwrap();
        assert_eq!(1, res.messages.len());
        let delegate = &res.messages[0];
        match delegate {
            CosmosMsg::Staking(StakingMsg::Undelegate { validator, amount }) => {
                assert_eq!(validator.as_str(), DEFAULT_VALIDATOR);
                assert_eq!(amount, &coin(bobs_claim.u128(), "ustake"));
            }
            _ => panic!("Unexpected message: {:?}", delegate),
        }

        // update the querier with new bond, lower balance
        set_delegation(&mut deps.querier, 690, "ustake");

        // check balances
        assert_eq!(get_balance(&deps, &bob), bobs_balance);
        assert_eq!(get_balance(&deps, &creator), owner_cut);
        // proper claims
        assert_eq!(get_claims(&deps, &bob), bobs_claim);

        // supplies updated, ratio the same (1.5)
        let ratio = Decimal::from_str("1.5").unwrap();

        let invest = query_investment(&deps).unwrap();
        assert_eq!(invest.token_supply, bobs_balance + owner_cut);
        assert_eq!(invest.staked_tokens, coin(690, "ustake")); // 1500 - 810
        assert_eq!(invest.nominal_value, ratio);
    }

    #[test]
    fn select_best_validator() {
        let mut deps = mock_dependencies(&[]);
        deps.querier.update_staking(
            "ustake",
            &[
                custom_sample_validator("john", 1, 10, 5),
                custom_sample_validator("mary", 2, 10, 1),
                custom_sample_validator("my-validator", 1, 10, 3),
            ],
            &[],
        );
        let validator = select_validator(&mut deps).unwrap();

        assert_eq!(validator, custom_sample_validator("my-validator", 1, 10, 3));
    }

    #[test]
    fn manage_delegators_list() {
        let mut deps = mock_dependencies(&[]);
        #[warn(unused_must_use)]
        let _ = delegators(&mut deps.storage).save(&vec![HumanAddr::from("init")]);
        let _ = delegators(&mut deps.storage).update(|mut delegator_list| -> StdResult<_> {
            delegator_list.append(&mut vec![HumanAddr::from("test")]);
            Ok(delegator_list)
        });
        let all_addr = query_all_delegators(&deps);

        assert_eq!(
            all_addr.unwrap(),
            vec![HumanAddr::from("init"), HumanAddr::from("test")]
        );
    }
}
