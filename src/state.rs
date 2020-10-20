use cosmwasm_std::{CanonicalAddr, Storage};
use cosmwasm_storage::{singleton, singleton_read, ReadonlySingleton, Singleton};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub static CONFIG_KEY: &[u8] = b"conifg";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
  pub okurinushi: CanonicalAddr,
  pub uketorinin: CanonicalAddr,
  pub tyokingaku: i32,
}

pub fn config<S: Storage>(storage: &mut S) -> Singleton<S, State> {
  singleton(storage, CONFIG_KEY)
}

pub fn config_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, State> {
  singleton_read(storage, CONFIG_KEY)
}
