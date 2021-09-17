use crate::keys::IntKey;
use cosmwasm_std::{Addr, StdError, StdResult};
use std::array::TryFromSliceError;
use std::convert::TryInto;

pub trait Deserializable {
    type Output: Sized;

    fn from_slice(value: &[u8]) -> StdResult<Self::Output>;
}

macro_rules! string_de {
    (for $($t:ty),+) => {
        $(impl Deserializable for $t {
            type Output = String;

            fn from_slice(value: &[u8]) -> StdResult<Self::Output> {
                // FIXME?: Use `from_utf8_unchecked` for String, &str
                String::from_utf8(value.to_vec())
                    // FIXME: Add and use StdError utf-8 error From helper
                    .map_err(|err| StdError::generic_err(err.to_string()))
            }
        })*
    }
}

// TODO: Confirm / extend these
string_de!(for String, &str, &[u8], Addr, &Addr);

macro_rules! integer_de {
    (for $($t:ty),+) => {
        $(impl Deserializable for IntKey<$t> {
            type Output = $t;

            fn from_slice(value: &[u8]) -> StdResult<Self::Output> {
                Ok(<$t>::from_be_bytes(value.try_into().map_err(|err: TryFromSliceError| StdError::generic_err(err.to_string()))?))
            }
        })*
    }
}

integer_de!(for i8, u8, i16, u16, i32, u32, i64, u64, i128, u128);
