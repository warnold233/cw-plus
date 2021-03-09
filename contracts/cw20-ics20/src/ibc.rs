#![allow(clippy::field_reassign_with_default)] // see https://github.com/CosmWasm/cosmwasm/issues/685

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{
    attr, entry_point, from_binary, to_binary, BankMsg, Binary, CosmosMsg, DepsMut, Env, HumanAddr,
    IbcAcknowledgement, IbcBasicResponse, IbcChannel, IbcOrder, IbcPacket, IbcReceiveResponse,
    StdResult, Uint128, WasmMsg,
};

use crate::amount::Amount;
use crate::error::ContractError;
use crate::state::{ChannelInfo, CHANNEL_INFO, CHANNEL_STATE};
use cw20::Cw20HandleMsg;

pub const ICS20_VERSION: &str = "ics20-1";
pub const ICS20_ORDERING: IbcOrder = IbcOrder::Unordered;

/// The format for sending an ics20 packet.
/// Proto defined here: https://github.com/cosmos/cosmos-sdk/blob/v0.42.0/proto/ibc/applications/transfer/v1/transfer.proto#L11-L20
/// This is compatible with the JSON serialization
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug, Default)]
pub struct Ics20Packet {
    // the token denomination to be transferred
    pub denom: String,
    // TODO: is this encoded as a string?
    pub amount: u64,
    // the sender address
    pub sender: String,
    // the recipient address on the destination chain
    pub receiver: String,
}

/// This is a generic ICS acknowledgement format.
/// Proto defined here: https://github.com/cosmos/cosmos-sdk/blob/v0.42.0/proto/ibc/core/channel/v1/channel.proto#L141-L147
/// This is compatible with the JSON serialization
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Ics20Ack {
    Result(Binary),
    Error(String),
}

// create a serialize success message
fn ack_success() -> StdResult<Binary> {
    let res = Ics20Ack::Result(b"1".into());
    to_binary(&res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// enforces ordering and versioning constraints
pub fn ibc_channel_open(
    _deps: DepsMut,
    _env: Env,
    channel: IbcChannel,
) -> Result<(), ContractError> {
    enforce_order_and_version(&channel)?;
    Ok(())
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// record the channel in CHANNEL_INFO
pub fn ibc_channel_connect(
    deps: DepsMut,
    _env: Env,
    channel: IbcChannel,
) -> Result<IbcBasicResponse, ContractError> {
    // we need to check the counter party version in try and ack (sometimes here)
    enforce_order_and_version(&channel)?;

    let info = ChannelInfo {
        id: channel.endpoint.channel_id,
        counterparty_endpoint: channel.counterparty_endpoint,
        connection_id: channel.connection_id,
    };
    CHANNEL_INFO.save(deps.storage, &info.id, &info)?;

    // TODO: add events/attributes here?
    let res = IbcBasicResponse::default();
    Ok(res)
}

fn enforce_order_and_version(channel: &IbcChannel) -> Result<(), ContractError> {
    if channel.version != ICS20_VERSION {
        return Err(ContractError::InvalidIbcVersion {
            version: channel.version.clone(),
        });
    }
    if let Some(version) = &channel.counterparty_version {
        if version != ICS20_VERSION {
            return Err(ContractError::InvalidIbcVersion {
                version: version.clone(),
            });
        }
    }
    if channel.order != ICS20_ORDERING {
        return Err(ContractError::OnlyOrderedChannel {});
    }
    Ok(())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _channel: IbcChannel,
) -> Result<IbcBasicResponse, ContractError> {
    // TODO: what to do here?
    // we will have locked funds that need to be returned somehow
    unimplemented!();
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// Check to see if we have any balance here
/// We should not return an error if possible, but rather an acknowledgement of failure
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    packet: IbcPacket,
) -> Result<IbcReceiveResponse, ContractError> {
    // TODO: don't let error leak
    let msg: Ics20Packet = from_binary(&packet.data)?;
    let channel = packet.src.channel_id;
    let denom = msg.denom;
    let amount = Uint128::from(msg.amount);
    CHANNEL_STATE.update(
        deps.storage,
        (&channel, &denom),
        |orig| -> Result<_, ContractError> {
            // this will return error if we don't have the funds there to cover the request (or no denom registered)
            let mut cur = orig.ok_or(ContractError::InsufficientFunds {})?;
            cur.outstanding = (cur.outstanding - amount)?;
            Ok(cur)
        },
    )?;

    // if we have funds, now send the tokens to the requested recipient
    let to_send = Amount::from_parts(denom, amount);
    let msg = send_amount(to_send, HumanAddr::from(msg.receiver))?;
    let res = IbcReceiveResponse {
        acknowledgement: ack_success()?,
        messages: vec![msg],
        // TODO: similar event messages like ibctransfer module
        attributes: vec![attr("action", "receive")],
    };
    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// check if success or failure and update balance, or return funds
pub fn ibc_packet_ack(
    deps: DepsMut,
    _env: Env,
    ack: IbcAcknowledgement,
) -> Result<IbcBasicResponse, ContractError> {
    // TODO: don't let error leak
    let msg: Ics20Ack = from_binary(&ack.acknowledgement)?;
    match msg {
        Ics20Ack::Result(_) => on_packet_success(deps, ack.original_packet),
        Ics20Ack::Error(err) => on_packet_failure(deps, ack.original_packet, err),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// return fund to original sender (same as failure in ibc_packet_ack)
pub fn ibc_packet_timeout(
    deps: DepsMut,
    _env: Env,
    packet: IbcPacket,
) -> Result<IbcBasicResponse, ContractError> {
    // TODO: don't let error leak
    on_packet_failure(deps, packet, "timeout".to_string())
}

// update the balance stored on this (channel, denom) index
fn on_packet_success(deps: DepsMut, packet: IbcPacket) -> Result<IbcBasicResponse, ContractError> {
    let msg: Ics20Packet = from_binary(&packet.data)?;
    let channel = packet.src.channel_id;
    let denom = msg.denom;
    let amount = Uint128::from(msg.amount);
    CHANNEL_STATE.update(deps.storage, (&channel, &denom), |orig| -> StdResult<_> {
        let mut state = orig.unwrap_or_default();
        state.outstanding += amount;
        state.total_sent += amount;
        Ok(state)
    })?;
    // TODO: similar event messages like ibctransfer module
    Ok(IbcBasicResponse::default())
}

// return the tokens to sender
fn on_packet_failure(
    _deps: DepsMut,
    packet: IbcPacket,
    err: String,
) -> Result<IbcBasicResponse, ContractError> {
    let msg: Ics20Packet = from_binary(&packet.data)?;

    let amount = Amount::from_parts(msg.denom, msg.amount.into());
    let msg = send_amount(amount, HumanAddr::from(msg.sender))?;
    let res = IbcBasicResponse {
        messages: vec![msg],
        // TODO: similar event messages like ibctransfer module
        attributes: vec![attr("ibc_error", err)],
    };
    Ok(res)
}

fn send_amount(amount: Amount, recipient: HumanAddr) -> StdResult<CosmosMsg> {
    match amount {
        Amount::Native(coin) => Ok(BankMsg::Send {
            to_address: recipient,
            amount: vec![coin],
        }
        .into()),
        Amount::Cw20(coin) => {
            let msg = Cw20HandleMsg::Transfer {
                recipient,
                amount: coin.amount,
            };
            let exec = WasmMsg::Execute {
                contract_addr: coin.address,
                msg: to_binary(&msg)?,
                send: vec![],
            };
            Ok(exec.into())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_helpers::*;

    use cosmwasm_std::testing::{mock_env, mock_info};
    use cosmwasm_std::to_vec;

    #[test]
    fn check_ack_json() {
        let success = Ics20Ack::Result(b"1".into());
        let fail = Ics20Ack::Error("bad coin".into());

        let success_json = String::from_utf8(to_vec(&success).unwrap()).unwrap();
        assert_eq!(r#"{"result":"MQ=="}"#, success_json.as_str());

        let fail_json = String::from_utf8(to_vec(&fail).unwrap()).unwrap();
        assert_eq!(r#"{"error":"bad coin"}"#, fail_json.as_str());
    }

    #[test]
    fn setup_and_query() {
        let deps = setup(&["channel-3", "channel-7"]);
    }
}
