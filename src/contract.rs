use cosmwasm_std::{
  log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
  HandleResponse, HandleResult, InitResponse, InitResult, Querier, StdError, StdResult, Storage, StakingQuery, QueryRequest
};

use crate::msg::{ArbiterResponse, ValidatorsResponse, HandleMsg, InitMsg, QueryMsg};
use crate::state::{config, config_read, State};

pub fn init<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  msg: InitMsg,
) -> InitResult {
  let state = State {
      arbiter: deps.api.canonical_address(&msg.arbiter)?,
      recipient: deps.api.canonical_address(&msg.recipient)?,
      source: deps.api.canonical_address(&env.message.sender)?,
      end_height: msg.end_height,
      end_time: msg.end_time,
  };
  if state.is_expired(&env) {
      Err(StdError::generic_err("creating expired escrow"))
  } else {
      config(&mut deps.storage).save(&state)?;
      Ok(InitResponse::default())
  }
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  msg: HandleMsg,
) -> HandleResult {
  let state = config_read(&deps.storage).load()?;
  match msg {
      HandleMsg::Approve { quantity } => try_approve(deps, env, state, quantity),
      HandleMsg::Refund {} => try_refund(deps, env, state),
  }
}

fn try_approve<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  state: State,
  quantity: Option<Vec<Coin>>,
) -> HandleResult {
  if deps.api.canonical_address(&env.message.sender)? != state.arbiter {
      Err(StdError::unauthorized())
  } else if state.is_expired(&env) {
      Err(StdError::generic_err("escrow expired"))
  } else {
      let amount = if let Some(quantity) = quantity {
          quantity
      } else {
          // release everything

          // Querier guarantees to returns up-to-date data, including funds sent in this handle message
          // https://github.com/CosmWasm/wasmd/blob/master/x/wasm/internal/keeper/keeper.go#L185-L192
          deps.querier.query_all_balances(&env.contract.address)?
      };

      send_tokens(
          &deps.api,
          &deps.api.canonical_address(&env.contract.address)?,
          &state.recipient,
          amount,
          "approve",
      )
  }
}

fn try_refund<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  state: State,
) -> HandleResult {
  // anyone can try to refund, as long as the contract is expired
  if !state.is_expired(&env) {
      Err(StdError::generic_err("escrow not yet expired"))
  } else {
      // Querier guarantees to returns up-to-date data, including funds sent in this handle message
      // https://github.com/CosmWasm/wasmd/blob/master/x/wasm/internal/keeper/keeper.go#L185-L192
      let balance = deps.querier.query_all_balances(&env.contract.address)?;
      send_tokens(
          &deps.api,
          &deps.api.canonical_address(&env.contract.address)?,
          &state.source,
          balance,
          "refund",
      )
  }
}

// this is a helper to move the tokens, so the business logic is easy to read
fn send_tokens<A: Api>(
  api: &A,
  from_address: &CanonicalAddr,
  to_address: &CanonicalAddr,
  amount: Vec<Coin>,
  action: &str,
) -> HandleResult {
  let from_human = api.human_address(from_address)?;
  let to_human = api.human_address(to_address)?;
  let log = vec![log("action", action), log("to", to_human.as_str())];

  let r = HandleResponse {
      messages: vec![CosmosMsg::Bank(BankMsg::Send {
          from_address: from_human,
          to_address: to_human,
          amount,
      })],
      log,
      data: None,
  };
  Ok(r)
}

pub fn query<S: Storage, A: Api, Q: Querier>(
  deps: &Extern<S, A, Q>,
  msg: QueryMsg,
) -> StdResult<Binary> {
  match msg {
      QueryMsg::Arbiter {} => to_binary(&query_arbiter(deps)?),
      QueryMsg::Validator {} => to_binary(&query_validators(deps)?),
  }
}

fn query_arbiter<S: Storage, A: Api, Q: Querier>(
  deps: &Extern<S, A, Q>,
) -> StdResult<ArbiterResponse> {
  let state = config_read(&deps.storage).load()?;
  let addr = deps.api.human_address(&state.arbiter)?;
  Ok(ArbiterResponse { arbiter: addr })
}

fn query_validators<S: Storage, A: Api, Q: Querier>(
  deps: &Extern<S, A, Q>,
) -> HandleResult {
  let res = deps.querier.query(&QueryRequest::Staking(StakingQuery::Validators {}))?;
  Ok(ValidatorsResponse { validators: res })
}
