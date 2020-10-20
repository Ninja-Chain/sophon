use cosmwasm_std::{
  attr, to_binary, Api, BankMsg, Binary, Coin, CosmosMsg, Env, Extern, HandleResponse, HumanAddr,
  InitResponse, MessageInfo, Querier, StdResult, Storage,
};

use crate::error::ContractError;
use crate::msg::{ZandakaResponse, HandleMsg, InitMsg, QueryMsg};
use crate::state::{config, config_read, State};

pub fn init<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  _env: Env,
  info: MessageInfo,
  msg: InitMsg,
) -> StdResult<InitResponse> {
  let state = State {
    okurinushi: deps.api.canonical_address(&info.sender)?,
    uketorinin: deps.api.canonical_address(&info.sender)?,
    tyokingaku: msg.tyokingaku,
  };

  config(&mut deps.storage).save(&state)?;

  Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  info: MessageInfo,
  msg: HandleMsg,
) -> Result<HandleResponse, ContractError> {
  let state = config_read(&deps.storage).load()?;
  match msg {
    HandleMsg::Tameru{ okurinushi, soukingaku } => try_tameru(deps, env, state, info, okurinushi, soukingaku),
    HandleMsg::Okuru{ uketorinin } => try_okuru(deps, env, state, info, uketorinin),
  }
}

fn try_tameru<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  _env: Env,
  _state: State,
  info: MessageInfo,
  _okurinushi: HumanAddr,
  soukingaku: i32,
) -> Result<HandleResponse, ContractError> {
  let state = config_read(&deps.storage).load()?;
  let state = State {
    okurinushi: deps.api.canonical_address(&info.sender)?,
    uketorinin: state.uketorinin,
    tyokingaku: state.tyokingaku + soukingaku,
  };

  config(&mut deps.storage).save(&state)?;

  Ok(HandleResponse::default())
}

fn try_okuru<S: Storage, A: Api, Q: Querier>(
  deps: &mut Extern<S, A, Q>,
  env: Env,
  _state: State,
  _info: MessageInfo,
  okurinushi: HumanAddr,
) -> Result<HandleResponse, ContractError> {
  let balance = deps.querier.query_all_balances(&env.contract.address)?;
  send_tokens(
    env.contract.address,
    okurinushi,
    balance,
    "okuru"
  )
}

// this is a helper to move the tokens, so the business logic is easy to read
fn send_tokens(
  from_address: HumanAddr,
  to_address: HumanAddr,
  amount: Vec<Coin>,
  action: &str,
) -> Result<HandleResponse, ContractError> {
  let attributes = vec![attr("action", action), attr("to", to_address.clone())];

  let r = HandleResponse {
      messages: vec![CosmosMsg::Bank(BankMsg::Send {
          from_address,
          to_address,
          amount,
      })],
      data: None,
      attributes,
  };
  Ok(r)
}


pub fn query<S: Storage, A: Api, Q: Querier>(
  deps: &Extern<S, A, Q>,
  _env: Env,
  msg: QueryMsg,
) -> StdResult<Binary> {
  match msg {
    QueryMsg::Zandaka {} => to_binary(&query_zandaka(deps)?),
  }
}

fn query_zandaka<S: Storage, A: Api, Q: Querier>(
  deps: &Extern<S, A, Q>,
) -> StdResult<ZandakaResponse> {
  let state = config_read(&deps.storage).load()?;
  Ok(ZandakaResponse { zandaka: state.tyokingaku })
}
