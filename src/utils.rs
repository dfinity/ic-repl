use anyhow::anyhow;
use candid::parser::value::IDLValue;

pub fn stringify(v: &IDLValue) -> anyhow::Result<String> {
    Ok(match v {
        IDLValue::Text(str) => str.to_string(),
        IDLValue::Number(n) => n.to_string(),
        IDLValue::Int64(n) => n.to_string(),
        IDLValue::Int32(n) => n.to_string(),
        IDLValue::Int16(n) => n.to_string(),
        IDLValue::Int8(n) => n.to_string(),
        IDLValue::Nat64(n) => n.to_string(),
        IDLValue::Nat32(n) => n.to_string(),
        IDLValue::Nat16(n) => n.to_string(),
        IDLValue::Nat8(n) => n.to_string(),
        IDLValue::Nat(n) => n.to_string(),
        IDLValue::Int(n) => n.to_string(),
        IDLValue::Principal(id) => id.to_string(),
        IDLValue::Bool(b) => b.to_string(),
        _ => return Err(anyhow!("Cannot stringify {}", v)),
    })
}
