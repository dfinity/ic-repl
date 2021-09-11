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
pub enum Exp {
    Path(String, Vec<Selector>),
    Blob(String),
    AnnVal(Box<Exp>, Type),
    Call {
        method: Option<Method>,
        args: Vec<Exp>,
        mode: CallMode,
    },
    Decode {
        method: Option<Method>,
        blob: Box<Exp>,
    },
    Fail(Box<Exp>),
    // from IDLValue without the infered types + Nat8
    Bool(bool),
    Null,
    Text(String),
    Number(String), // Undetermined number type
    Nat8(u8),
    Float64(f64),
    Opt(Box<Exp>),
    Vec(Vec<Exp>),
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
pub enum CallMode {
    Call,
    Encode,
    Proxy(String),
}
#[derive(Debug, Clone)]
pub struct Field {
    pub id: Label,
    pub val: Exp,
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
impl Exp {
    pub fn eval(self, helper: &MyHelper) -> Result<IDLValue> {
        Ok(match self {
            Exp::Path(id, path) => {
                let v = helper
                    .env
                    .0
                    .get(&id)
                    .ok_or_else(|| anyhow!("Undefined variable {}", id))?;
                project(v, &path)?.clone()
            }
            Exp::Blob(file) => {
                let path = resolve_path(&helper.base_path, &file);
                let blob: Vec<IDLValue> = std::fs::read(&path)
                    .with_context(|| format!("Cannot read {:?}", path))?
                    .into_iter()
                    .map(IDLValue::Nat8)
                    .collect();
                IDLValue::Vec(blob)
            }
            Exp::AnnVal(v, ty) => {
                let arg = v.eval(helper)?;
                let env = TypeEnv::new();
                arg.annotate_type(true, &env, &ty)?
            }
            Exp::Fail(v) => match v.eval(helper) {
                Err(e) => IDLValue::Text(e.to_string()),
                Ok(_) => return Err(anyhow!("Expect an error state")),
            },
            Exp::Decode { method, blob } => {
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
                        let (_, func) = method.get_info(helper)?;
                        if let Some((env, func)) = func {
                            IDLArgs::from_bytes_with_types(&bytes, &env, &func.rets)?
                        } else {
                            IDLArgs::from_bytes(&bytes)?
                        }
                    }
                    None => IDLArgs::from_bytes(&bytes)?,
                };
                args_to_value(args)
            }
            Exp::Call { method, args, mode } => {
                let mut res = Vec::with_capacity(args.len());
                for arg in args.into_iter() {
                    res.push(arg.eval(helper)?);
                }
                let args = IDLArgs { args: res };
                let opt_func = if let Some(method) = &method {
                    Some(method.get_info(helper)?)
                } else {
                    None
                };
                let bytes = if let Some((_, Some((env, func)))) = &opt_func {
                    args.to_bytes_with_types(env, &func.args)?
                } else {
                    args.to_bytes()?
                };
                match mode {
                    CallMode::Encode => {
                        IDLValue::Vec(bytes.into_iter().map(IDLValue::Nat8).collect())
                    }
                    CallMode::Call => {
                        let method = method.unwrap(); // okay to unwrap from parser
                        let (canister_id, opt_func) = opt_func.unwrap();
                        let res = call(
                            &helper.agent,
                            &canister_id,
                            &method.method,
                            &bytes,
                            &opt_func,
                            helper.offline,
                        )?;
                        args_to_value(res)
                    }
                    CallMode::Proxy(id) => {
                        let method = method.unwrap();
                        let canister_id = str_to_principal(&method.canister, helper)?;
                        let proxy_id = str_to_principal(&id, helper)?;
                        let mut env = MyHelper::new(
                            helper.agent.clone(),
                            helper.agent_url.clone(),
                            helper.offline,
                        );
                        env.canister_map.borrow_mut().0.insert(
                            proxy_id,
                            helper
                                .canister_map
                                .borrow()
                                .0
                                .get(&proxy_id)
                                .ok_or_else(|| {
                                    anyhow!("{} canister interface not found", proxy_id)
                                })?
                                .clone(),
                        );
                        env.env.0.insert(
                            "_msg".to_string(),
                            IDLValue::Vec(bytes.into_iter().map(IDLValue::Nat8).collect()),
                        );
                        let code = format!(
                            r#"
let _ = call "{id}".wallet_call(
  record {{
    args = _msg;
    cycles = 0;
    method_name = "{method}";
    canister = principal "{canister}";
  }}
);
let _ = decode as "{canister}".{method} _.Ok.return;
"#,
                            id = proxy_id,
                            canister = canister_id,
                            method = method.method
                        );
                        let cmds =
                            crate::pretty_parse::<crate::command::Commands>("forward_call", &code)?;
                        for cmd in cmds.0.into_iter() {
                            cmd.run(&mut env)?;
                        }
                        env.env.0.get("_").unwrap().clone()
                    }
                }
            }
            Exp::Bool(b) => IDLValue::Bool(b),
            Exp::Null => IDLValue::Null,
            Exp::Text(s) => IDLValue::Text(s),
            Exp::Nat8(n) => IDLValue::Nat8(n),
            Exp::Number(n) => IDLValue::Number(n),
            Exp::Float64(f) => IDLValue::Float64(f),
            Exp::Principal(id) => IDLValue::Principal(id),
            Exp::Service(id) => IDLValue::Service(id),
            Exp::Func(id, meth) => IDLValue::Func(id, meth),
            Exp::Opt(v) => IDLValue::Opt(Box::new((*v).eval(helper)?)),
            Exp::Vec(vs) => {
                let mut vec = Vec::with_capacity(vs.len());
                for v in vs.into_iter() {
                    vec.push(v.eval(helper)?);
                }
                IDLValue::Vec(vec)
            }
            Exp::Record(fs) => {
                let mut res = Vec::with_capacity(fs.len());
                for Field { id, val } in fs.into_iter() {
                    res.push(IDLField {
                        id,
                        val: val.eval(helper)?,
                    });
                }
                IDLValue::Record(res)
            }
            Exp::Variant(f, idx) => {
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

impl std::str::FromStr for Exp {
    type Err = ParserError;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let lexer = Tokenizer::new(str);
        super::grammar::ExpParser::new().parse(lexer)
    }
}
pub fn str_to_principal(id: &str, helper: &MyHelper) -> Result<Principal> {
    let try_id = Principal::from_text(id);
    Ok(match try_id {
        Ok(id) => id,
        Err(_) => match helper.env.0.get(id) {
            Some(IDLValue::Principal(id)) => *id,
            _ => return Err(anyhow!("{} is not a canister id", id)),
        },
    })
}
fn get_type(
    canister_id: Principal,
    method: &str,
    helper: &MyHelper,
) -> Option<(TypeEnv, Function)> {
    let agent = &helper.agent;
    let mut map = helper.canister_map.borrow_mut();
    let info = map.get(agent, &canister_id).ok()?;
    let func = info.methods.get(method)?.clone();
    Some((info.env.clone(), func))
}
impl Method {
    fn get_info(&self, helper: &MyHelper) -> Result<(Principal, Option<(TypeEnv, Function)>)> {
        let canister_id = str_to_principal(&self.canister, helper)?;
        let func = get_type(canister_id, &self.method, helper);
        if func.is_none() {
            eprintln!(
                "Warning: cannot get type for {}.{}, use types infered from textual value",
                self.canister, self.method
            );
        }
        Ok((canister_id, func))
    }
}

#[derive(serde::Serialize)]
struct Ingress {
    call_type: String,
    request_id: Option<String>,
    content: String,
}
#[derive(serde::Serialize)]
struct RequestStatus {
    canister_id: Principal,
    request_id: String,
    content: String,
}
#[derive(serde::Serialize)]
struct IngressWithStatus {
    ingress: Ingress,
    request_status: RequestStatus,
}

fn qrcode_gen(json: String) -> anyhow::Result<String> {
    use qrcode::{render::unicode, QrCode};
    let code = QrCode::new(&json)?;
    let img = code.render::<unicode::Dense1x2>().build();
    Ok(img)
}

#[tokio::main]
async fn call(
    agent: &Agent,
    canister_id: &Principal,
    method: &str,
    args: &[u8],
    opt_func: &Option<(TypeEnv, Function)>,
    offline: bool,
) -> anyhow::Result<IDLArgs> {
    let effective_id = get_effective_canister_id(*canister_id, method, args)?;
    let is_query = opt_func
        .as_ref()
        .map(|(_, f)| f.is_query())
        .unwrap_or(false);
    let bytes = if is_query {
        let mut builder = agent.query(canister_id, method);
        builder
            .with_arg(args)
            .with_effective_canister_id(effective_id);
        if offline {
            let signed = builder.sign()?;
            let message = Ingress {
                call_type: "query".to_owned(),
                request_id: None,
                content: hex::encode(signed.signed_query),
            };
            let image = qrcode_gen(serde_json::to_string(&message)?)?;
            println!("{}", image);
            return Ok(IDLArgs::new(&[]));
        } else {
            builder.call().await?
        }
    } else {
        let mut builder = agent.update(canister_id, method);
        builder
            .with_arg(args)
            .with_effective_canister_id(effective_id);
        if offline {
            let signed = builder.sign()?;
            let status = agent.sign_request_status(effective_id, signed.request_id)?;
            let message = IngressWithStatus {
                ingress: Ingress {
                    call_type: "update".to_owned(),
                    request_id: Some(hex::encode(signed.request_id.as_slice())),
                    content: hex::encode(signed.signed_update),
                },
                request_status: RequestStatus {
                    canister_id: status.effective_canister_id,
                    request_id: hex::encode(status.request_id.as_slice()),
                    content: hex::encode(status.signed_request_status),
                },
            };
            let image = qrcode_gen(serde_json::to_string(&message)?)?;
            println!("{}", image);
            return Ok(IDLArgs::new(&[]));
        } else {
            let waiter = garcon::Delay::builder()
                .exponential_backoff(std::time::Duration::from_secs(1), 1.1)
                .timeout(std::time::Duration::from_secs(60 * 5))
                .build();
            builder.call_and_wait(waiter).await?
        }
    };
    let res = if let Some((env, func)) = opt_func {
        IDLArgs::from_bytes_with_types(&bytes, env, &func.rets)?
    } else {
        IDLArgs::from_bytes(&bytes)?
    };
    Ok(res)
}

fn get_effective_canister_id(
    canister_id: Principal,
    method: &str,
    args: &[u8],
) -> anyhow::Result<Principal> {
    use candid::{CandidType, Decode, Deserialize};
    if canister_id == Principal::management_canister() {
        match method {
            "create_canister" | "raw_rand" => Err(anyhow!(
                "{} can only be called via inter-canister call.",
                method
            )),
            "provisional_create_canister_with_cycles" => Ok(canister_id),
            _ => {
                #[derive(CandidType, Deserialize)]
                struct Arg {
                    canister_id: Principal,
                }
                let args = Decode!(args, Arg)?;
                Ok(args.canister_id)
            }
        }
    } else {
        Ok(canister_id)
    }
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
