use super::command::resolve_path;
use super::helper::MyHelper;
use super::token::{ParserError, Tokenizer};
use anyhow::{anyhow, Context, Result};
use candid::{
    parser::value::{IDLArgs, IDLField, IDLValue, VariantValue},
    types::{Function, Label, Type},
    Principal, TypeEnv,
};
use ic_agent::Agent;

#[derive(Debug, Clone)]
pub enum Value {
    Path(String, Vec<Selector>),
    Blob(String),
    AnnVal(Box<Value>, Type),
    Method {
        method: Option<Method>,
        args: Vec<Value>,
        encode_only: bool,
    },
    Decode {
        method: Option<Method>,
        blob: Box<Value>,
    },
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
pub struct Method {
    pub canister: String,
    pub method: String,
}
#[derive(Debug, Clone)]
pub struct Field {
    pub id: Label,
    pub val: Value,
}
#[derive(Debug, Clone)]
pub enum Selector {
    Index(u64),
    Field(String),
}
impl Selector {
    fn to_label(&self) -> Label {
        match self {
            Selector::Index(idx) => Label::Id(*idx as u32),
            Selector::Field(name) => Label::Named(name.to_string()),
        }
    }
}
impl Value {
    pub fn eval(self, helper: &MyHelper) -> Result<IDLValue> {
        Ok(match self {
            Value::Path(id, path) => {
                let v = helper
                    .env
                    .0
                    .get(&id)
                    .ok_or_else(|| anyhow!("Undefined variable {}", id))?;
                project(&v, &path)?.clone()
            }
            Value::Blob(file) => {
                let path = resolve_path(&helper.base_path, &file);
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
            Value::Decode { method, blob } => {
                let blob = blob.eval(helper)?;
                if blob.value_ty() != Type::Vec(Box::new(Type::Nat8)) {
                    return Err(anyhow!("not a blob"));
                }
                let bytes: Vec<u8> = match blob {
                    IDLValue::Vec(vs) => vs
                        .into_iter()
                        .map(|v| match v {
                            IDLValue::Nat8(u) => u,
                            _ => unreachable!(),
                        })
                        .collect(),
                    _ => unreachable!(),
                };
                let args = match method {
                    Some(method) => {
                        let (_, env, func) = method.get_type(helper)?;
                        IDLArgs::from_bytes_with_types(&bytes, &env, &func.rets)?
                    }
                    None => IDLArgs::from_bytes(&bytes)?,
                };
                args_to_value(args)
            }
            Value::Method {
                method,
                args,
                encode_only,
            } => {
                let mut res = Vec::with_capacity(args.len());
                for arg in args.into_iter() {
                    res.push(arg.eval(helper)?);
                }
                let args = IDLArgs { args: res };
                let opt_func = if let Some(method) = &method {
                    Some(method.get_type(helper)?)
                } else {
                    None
                };
                let bytes = if let Some((_, env, func)) = &opt_func {
                    args.to_bytes_with_types(&env, &func.args)?
                } else {
                    args.to_bytes()?
                };
                if encode_only {
                    IDLValue::Vec(bytes.into_iter().map(IDLValue::Nat8).collect())
                } else {
                    let method = method.unwrap(); // okay to unwrap from parser
                    let (canister_id, env, func) = opt_func.unwrap();
                    let res = call(
                        &helper.agent,
                        &canister_id,
                        &method.method,
                        &bytes,
                        &env,
                        &func,
                    )?;
                    args_to_value(res)
                }
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

pub fn project<'a>(value: &'a IDLValue, path: &[Selector]) -> Result<&'a IDLValue> {
    if path.is_empty() {
        return Ok(value);
    }
    let (head, tail) = (&path[0], &path[1..]);
    match (value, head) {
        (IDLValue::Opt(opt), Selector::Field(f)) if f == "?" => return project(&*opt, tail),
        (IDLValue::Vec(vs), Selector::Index(idx)) => {
            let idx = *idx as usize;
            if idx < vs.len() {
                return project(&vs[idx], tail);
            }
        }
        (IDLValue::Record(fs), field) => {
            let id = field.to_label();
            if let Some(v) = fs.iter().find(|f| f.id == id) {
                return project(&v.val, tail);
            }
        }
        (IDLValue::Variant(VariantValue(f, _)), field) => {
            if field.to_label() == f.id {
                return project(&f.val, tail);
            }
        }
        _ => (),
    }
    return Err(anyhow!("{:?} cannot be applied to {}", head, value));
}

impl std::str::FromStr for Value {
    type Err = ParserError;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let lexer = Tokenizer::new(str);
        super::grammar::ValueParser::new().parse(lexer)
    }
}
impl Method {
    fn get_type(&self, helper: &MyHelper) -> Result<(Principal, TypeEnv, Function)> {
        let try_id = Principal::from_text(&self.canister);
        let canister_id = match try_id {
            Ok(ref id) => id,
            Err(_) => match helper.env.0.get(&self.canister) {
                Some(IDLValue::Principal(id)) => id,
                _ => return Err(anyhow!("{} is not a canister id", self.canister)),
            },
        };
        let agent = &helper.agent;
        let mut map = helper.canister_map.borrow_mut();
        let info = map.get(&agent, &canister_id)?;
        let func = info
            .methods
            .get(&self.method)
            .ok_or_else(|| anyhow!("no method {}", self.method))?
            .clone();
        // TODO remove clone
        Ok((canister_id.clone(), info.env.clone(), func))
    }
}

#[tokio::main]
async fn call(
    agent: &Agent,
    canister_id: &Principal,
    method: &str,
    args: &[u8],
    env: &TypeEnv,
    func: &Function,
) -> anyhow::Result<IDLArgs> {
    let bytes = if func.is_query() {
        agent
            .query(canister_id, method)
            .with_arg(args)
            .with_effective_canister_id(canister_id.clone())
            .call()
            .await?
    } else {
        let waiter = delay::Delay::builder()
            .exponential_backoff(std::time::Duration::from_secs(1), 1.1)
            .timeout(std::time::Duration::from_secs(60 * 5))
            .build();
        agent
            .update(canister_id, method)
            .with_arg(args)
            .with_effective_canister_id(canister_id.clone())
            .call_and_wait(waiter)
            .await?
    };
    Ok(IDLArgs::from_bytes_with_types(&bytes, env, &func.rets)?)
}

fn args_to_value(mut args: IDLArgs) -> IDLValue {
    match args.args.len() {
        0 => IDLValue::Null,
        1 => args.args.pop().unwrap(),
        len => {
            let mut fs = Vec::with_capacity(len);
            for (i, v) in args.args.into_iter().enumerate() {
                fs.push(IDLField {
                    id: Label::Id(i as u32),
                    val: v,
                });
            }
            IDLValue::Record(fs)
        }
    }
}
