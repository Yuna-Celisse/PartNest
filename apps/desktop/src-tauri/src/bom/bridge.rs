//! Versioned, narrow message contract between the cached BOM iframe and Rust.

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

pub const MAX_SELECTION_JSON_BYTES: usize = 64 * 1024;
pub const MAX_DESIGNATORS: usize = 512;
pub const MAX_TOKEN_BYTES: usize = 128;
pub const MAX_DESIGNATOR_BYTES: usize = 64;

pub const BRIDGE_VERSION: &str = "bridge-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BomSelectionMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub token: String,
    pub designators: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    InvalidToken,
    InactiveSession,
    InvalidMessage(String),
    DuplicateDesignator,
    UnknownDesignator,
    CrossGroupSelection,
    MixedSideSelection,
    TooManyDesignators,
    DesignatorTooLong,
    MessageTooLarge,
    EmptyToken,
    TokenTooLong,
    EmptyDesignator,
}
impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToken => f.write_str("invalid BOM bridge token"),
            Self::InactiveSession => f.write_str("BOM session is not active"),
            Self::InvalidMessage(reason) => write!(f, "invalid BOM bridge message: {reason}"),
            Self::DuplicateDesignator => f.write_str("duplicate designator"),
            Self::UnknownDesignator => f.write_str("unknown designator"),
            Self::CrossGroupSelection => f.write_str("designators belong to different BOM groups"),
            Self::MixedSideSelection => f.write_str("designators belong to different board sides"),
            Self::TooManyDesignators => f.write_str("too many selected designators"),
            Self::DesignatorTooLong => f.write_str("designator is too long"),
            Self::MessageTooLarge => f.write_str("BOM bridge message is too large"),
            Self::EmptyToken => f.write_str("BOM bridge token is empty"),
            Self::TokenTooLong => f.write_str("BOM bridge token is too long"),
            Self::EmptyDesignator => f.write_str("designator is empty"),
        }
    }
}

impl BridgeError {
    /// Reader-facing text. The English `Display` form stays available for logs
    /// and assertions, while commands surface Chinese text in the UI.
    pub fn user_message(&self) -> String {
        match self {
            Self::InvalidToken => "BOM 会话令牌无效，请重新在 BOM 中选择器件".into(),
            Self::InactiveSession => "焊接会话已失效，请在「项目」页重新打开该项目的焊接".into(),
            Self::InvalidMessage(reason) => format!("BOM 选择消息无效：{reason}"),
            Self::DuplicateDesignator => "选中的位号有重复".into(),
            Self::UnknownDesignator => "BOM 返回了不属于当前 BOM 的位号".into(),
            Self::CrossGroupSelection => "选中的位号属于不同器件，请只选一个器件".into(),
            Self::MixedSideSelection => "选中的位号属于不同板面，请切换到单一板面后再选".into(),
            Self::TooManyDesignators => "一次选中的位号过多，请缩小选择范围".into(),
            Self::DesignatorTooLong => "位号长度超出限制".into(),
            Self::MessageTooLarge => "BOM 选择消息过大".into(),
            Self::EmptyToken => "BOM 会话令牌为空".into(),
            Self::TokenTooLong => "BOM 会话令牌长度超出限制".into(),
            Self::EmptyDesignator => "选中的位号为空".into(),
        }
    }
}

/// Selection validation errors are also bridge errors so command handlers can
/// expose one narrow rejection contract.
pub type SelectionError = BridgeError;
impl std::error::Error for BridgeError {}

pub fn decode_selection_message(json: &str) -> Result<BomSelectionMessage, BridgeError> {
    if json.len() > MAX_SELECTION_JSON_BYTES {
        return Err(BridgeError::MessageTooLarge);
    }
    let message: BomSelectionMessage = serde_json::from_str(json)
        .map_err(|error| BridgeError::InvalidMessage(error.to_string()))?;
    validate_selection_message(&message)?;
    Ok(message)
}

/// Enforce the bridge contract on an already decoded message. Commands that
/// receive a typed payload call this so the shape, size, and uniqueness limits
/// cannot be bypassed by a caller that skipped the JSON decode path.
pub fn validate_selection_message(message: &BomSelectionMessage) -> Result<(), BridgeError> {
    if message.message_type != "partnest:bom-selection" {
        return Err(BridgeError::InvalidMessage(
            "unexpected message type".into(),
        ));
    }
    validate_token_and_designators(&message.token, &message.designators)?;
    let unique = message.designators.iter().collect::<HashSet<_>>();
    if unique.len() != message.designators.len() {
        return Err(BridgeError::InvalidMessage("duplicate designator".into()));
    }
    Ok(())
}

pub(crate) fn validate_token_and_designators(
    token: &str,
    designators: &[String],
) -> Result<(), BridgeError> {
    if token.is_empty() {
        return Err(BridgeError::EmptyToken);
    }
    if token.len() > MAX_TOKEN_BYTES {
        return Err(BridgeError::TokenTooLong);
    }
    if designators.is_empty() {
        return Err(BridgeError::InvalidMessage(
            "designators are required".into(),
        ));
    }
    if designators.len() > MAX_DESIGNATORS {
        return Err(BridgeError::TooManyDesignators);
    }
    for designator in designators {
        if designator.is_empty() {
            return Err(BridgeError::EmptyDesignator);
        }
        if designator.trim() != designator {
            return Err(BridgeError::InvalidMessage(
                "designators must be trimmed".into(),
            ));
        }
        if designator.len() > MAX_DESIGNATOR_BYTES {
            return Err(BridgeError::DesignatorTooLong);
        }
    }
    Ok(())
}

pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    let max = left.len().max(right.len());
    for index in 0..max {
        difference |= usize::from(
            left.as_bytes().get(index).copied().unwrap_or(0)
                ^ right.as_bytes().get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}
