use super::command::resolve_path;
use super::helper::MyHelper;
use anyhow::{anyhow, Context};
use candid::{
    parser::value::{IDLField, IDLValue, VariantValue},
    types::{Label, Type},
    Principal, TypeEnv,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Value {
    Path(Vec<String>),
    Blob(String),
    AnnVal(Box<Value>, Type),
    // from IDLValue without the infered types + Nat8
    Bool(bool),
    Null,
    Text(String),
    Number(String), // Undetermined number type
    Nat8(u8),
    Float64(f64),
    Opt(Box<Value>),
    Vec(Vec<Value>),
    Record(Vec<Field>),
    Variant(Box<Field>, u64), // u64 represents the index from the type, defaults to 0 when parsing
    Principal(Principal),
    Service(Principal),
    Func(Principal, String),
}
#[derive(Debug, Clone)]
pub struct Field {
    pub id: Label,
    pub val: Value,
}
impl Value {
    pub fn eval(self, helper: &MyHelper) -> anyhow::Result<IDLValue> {
        Ok(match self {
            Value::Path(vs) => helper
                .env
                .0
                .get(&vs[0]) // TODO handle path
                .ok_or_else(|| anyhow!("Undefined variable {}", vs[0]))?
                .clone(),
            Value::Blob(file) => {
                let path = resolve_path(&helper.base_path, PathBuf::from(file));
                let blob: Vec<IDLValue> = std::fs::read(&path)
                    .with_context(|| format!("Cannot read {:?}", path))?
                    .into_iter()
                    .map(IDLValue::Nat8)
                    .collect();
                IDLValue::Vec(blob)
            }
            Value::AnnVal(v, ty) => {
                let arg = v.eval(helper)?;
                let env = TypeEnv::new();
                arg.annotate_type(true, &env, &ty)?
            }
            Value::Bool(b) => IDLValue::Bool(b),
            Value::Null => IDLValue::Null,
            Value::Text(s) => IDLValue::Text(s),
            Value::Nat8(n) => IDLValue::Nat8(n),
            Value::Number(n) => IDLValue::Number(n),
            Value::Float64(f) => IDLValue::Float64(f),
            Value::Principal(id) => IDLValue::Principal(id),
            Value::Service(id) => IDLValue::Service(id),
            Value::Func(id, meth) => IDLValue::Func(id, meth),
            Value::Opt(v) => IDLValue::Opt(Box::new((*v).eval(helper)?)),
            Value::Vec(vs) => {
                let mut vec = Vec::with_capacity(vs.len());
                for v in vs.into_iter() {
                    vec.push(v.eval(helper)?);
                }
                IDLValue::Vec(vec)
            }
            Value::Record(fs) => {
                let mut res = Vec::with_capacity(fs.len());
                for Field { id, val } in fs.into_iter() {
                    res.push(IDLField {
                        id,
                        val: val.eval(helper)?,
                    });
                }
                IDLValue::Record(res)
            }
            Value::Variant(f, idx) => {
                let f = IDLField {
                    id: f.id,
                    val: f.val.eval(helper)?,
                };
                IDLValue::Variant(VariantValue(Box::new(f), idx))
            }
        })
    }
}
