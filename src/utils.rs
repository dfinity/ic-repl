use anyhow::anyhow;
use candid::parser::value::IDLValue;
use std::borrow::Cow;

pub fn stringify(v: &IDLValue) -> anyhow::Result<Cow<str>> {
    Ok(match v {
        IDLValue::Text(str) => Cow::Borrowed(str),
        IDLValue::Number(n) => Cow::Owned(n.to_string()),
        IDLValue::Int64(n) => Cow::Owned(n.to_string()),
        IDLValue::Int32(n) => Cow::Owned(n.to_string()),
        IDLValue::Int16(n) => Cow::Owned(n.to_string()),
        IDLValue::Int8(n) => Cow::Owned(n.to_string()),
        IDLValue::Nat64(n) => Cow::Owned(n.to_string()),
        IDLValue::Nat32(n) => Cow::Owned(n.to_string()),
        IDLValue::Nat16(n) => Cow::Owned(n.to_string()),
        IDLValue::Nat8(n) => Cow::Owned(n.to_string()),
        IDLValue::Nat(n) => Cow::Owned(n.to_string()),
        IDLValue::Int(n) => Cow::Owned(n.to_string()),
        IDLValue::Principal(id) => Cow::Owned(id.to_string()),
        IDLValue::Bool(b) => Cow::Owned(b.to_string()),
        _ => return Err(anyhow!("Cannot stringify {}", v)),
    })
}
