use cosmwasm_std::{StdError, CanonicalAddr};
use thiserror::Error;

use cw_controllers::{AdminError, HookError};

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Admin(#[from] AdminError),

    #[error("{0}")]
    Hook(#[from] HookError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("No claims that can be released currently")]
    NothingToClaim {},

    #[error("Must send '{0}' to stake")]
    MissingDenom(String),

    #[error("Sent unsupported denoms, must send '{0}' to stake")]
    ExtraDenoms(String),

    #[error("Must send valid address to stake")]
    MissingAddress(CanonicalAddr),

    #[error("Sent unsupported addresses, must send canonical address to stake")]
    ExtraAddresses(CanonicalAddr),

    #[error("No funds sent")]
    NoFunds {},
}
