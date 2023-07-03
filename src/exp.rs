use super::error::pretty_parse;
use super::helper::{fetch_metadata, MyHelper, OfflineOutput};
use super::selector::{project, Selector};
use super::token::{ParserError, Tokenizer};
use super::utils::{
    args_to_value, cast_type, get_effective_canister_id, resolve_path, str_to_principal,
};
use anyhow::{anyhow, Context, Result};
use candid::{
    types::value::{IDLArgs, IDLField, IDLValue, VariantValue},
    types::{Function, Label, Type, TypeInner},
    utils::check_unique,
    Principal, TypeEnv,
};
use ic_agent::Agent;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Exp {
    Path(String, Vec<Selector>),
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
    Apply(String, Vec<Exp>),
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
impl Exp {
    pub fn is_call(&self) -> bool {
        matches!(
            self,
            Exp::Call {
                mode: CallMode::Call,
                ..
            }
        )
    }
    pub fn eval(self, helper: &MyHelper) -> Result<IDLValue> {
        Ok(match self {
            Exp::Path(id, path) => {
                let v = helper
                    .env
                    .0
                    .get(&id)
                    .ok_or_else(|| anyhow!("Undefined variable {}", id))?
                    .clone();
                project(helper, v, path)?
            }
            Exp::AnnVal(v, ty) => {
                let arg = v.eval(helper)?;
                cast_type(arg, &ty).with_context(|| format!("casting to type {ty} fails"))?
            }
            Exp::Fail(v) => match v.eval(helper) {
                Err(e) => IDLValue::Text(e.to_string()),
                Ok(_) => return Err(anyhow!("Expects an error state")),
            },
            Exp::Apply(func, exps) => {
                use crate::account_identifier::*;

                let mut args = Vec::new();
                for e in exps.into_iter() {
                    args.push(e.eval(helper)?);
                }
                match func.as_str() {
                    "account" => match args.as_slice() {
                        [IDLValue::Principal(principal)] => {
                            let account = AccountIdentifier::new(*principal, None);
                            IDLValue::Vec(
                                account.to_vec().into_iter().map(IDLValue::Nat8).collect(),
                            )
                        }
                        _ => return Err(anyhow!("account expects principal")),
                    },
                    "neuron_account" => match args.as_slice() {
                        [IDLValue::Principal(principal), nonce] => {
                            let nonce = match nonce {
                                IDLValue::Number(nonce) => nonce.parse::<u64>()?,
                                IDLValue::Nat64(nonce) => *nonce,
                                _ => {
                                    return Err(anyhow!(
                                        "neuron_account expects (principal, nonce)"
                                    ))
                                }
                            };
                            let nns = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")?;
                            let subaccount = get_neuron_subaccount(principal, nonce);
                            let account = AccountIdentifier::new(nns, Some(subaccount));
                            IDLValue::Vec(
                                account.to_vec().into_iter().map(IDLValue::Nat8).collect(),
                            )
                        }
                        _ => return Err(anyhow!("neuron_account expects (principal, nonce)")),
                    },
                    "metadata" => match args.as_slice() {
                        [IDLValue::Principal(id), IDLValue::Text(path)] => {
                            let res = fetch_metadata(&helper.agent, *id, path)?;
                            IDLValue::Vec(res.into_iter().map(IDLValue::Nat8).collect())
                        }
                        _ => return Err(anyhow!("metadata expects (principal, path)")),
                    }
                    "file" => match args.as_slice() {
                        [IDLValue::Text(file)] => {
                            let path = resolve_path(&helper.base_path, file);
                            let blob: Vec<IDLValue> = std::fs::read(&path)
                                .with_context(|| format!("Cannot read {path:?}"))?
                                .into_iter()
                                .map(IDLValue::Nat8)
                                .collect();
                            IDLValue::Vec(blob)
                        }
                        _ => return Err(anyhow!("file expects file path")),
                    },
                    "gzip" => match args.as_slice() {
                        [IDLValue::Vec(blob)] => {
                            use libflate::gzip::Encoder;
                            use std::io::Write;
                            let blob: Vec<u8> = blob
                                .iter()
                                .filter_map(|v| match v {
                                    IDLValue::Nat8(n) => Some(*n),
                                    _ => None,
                                })
                                .collect();
                            let mut encoder = Encoder::new(Vec::with_capacity(blob.len()))?;
                            encoder.write_all(&blob)?;
                            let result = encoder.finish().into_result()?;
                            IDLValue::Vec(result.into_iter().map(IDLValue::Nat8).collect())
                        }
                        _ => return Err(anyhow!("gzip expects blob")),
                    },
                    "wasm_profiling" => match args.as_slice() {
                        [IDLValue::Text(file)] | [IDLValue::Text(file), IDLValue::Vec(_)] => {
                            let path = resolve_path(&helper.base_path, file);
                            let blob = std::fs::read(&path)
                                .with_context(|| format!("Cannot read {path:?}"))?;
                            let mut m = ic_wasm::utils::parse_wasm(&blob, false)?;
                            ic_wasm::shrink::shrink(&mut m);
                            let trace_funcs: Vec<String> = match args.as_slice() {
                                [_] => vec![],
                                [_, IDLValue::Vec(vec)] => vec.iter().filter_map(|name| if let IDLValue::Text(name) = name { Some(name.clone()) } else { None }).collect(),
                                _ => unreachable!(),
                            };
                            ic_wasm::instrumentation::instrument(&mut m, &trace_funcs).map_err(|e| anyhow::anyhow!("{e}"))?;
                            IDLValue::Vec(m.emit_wasm().into_iter().map(IDLValue::Nat8).collect())
                        }
                        _ => return Err(anyhow!("wasm_profiling expects file path and optionally vec text of function names")),
                    },
                    "flamegraph" => match args.as_slice() {
                        [IDLValue::Principal(cid), IDLValue::Text(title), IDLValue::Text(file)] => {
                            let mut map = helper.canister_map.borrow_mut();
                            let names = match map.get(&helper.agent, cid) {
                                Ok(crate::helper::CanisterInfo {
                                    profiling: Some(names),
                                    ..
                                }) => names,
                                _ => return Err(anyhow!("{} is not instrumented", cid)),
                            };
                            let mut path = resolve_path(&std::env::current_dir()?, file);
                            if path.extension().is_none() {
                                path.set_extension("svg");
                            }
                            let cost = crate::profiling::get_profiling(
                                &helper.agent,
                                cid,
                                names,
                                title,
                                path,
                            )?;
                            IDLValue::Nat(cost.into())
                        }
                        _ => {
                            return Err(anyhow!(
                                "flamegraph expects (canister id, title name, svg file name)"
                            ))
                        }
                    },
                    "output" => match args.as_slice() {
                        [IDLValue::Text(file), IDLValue::Text(content)] => {
                            use std::fs::OpenOptions;
                            use std::io::Write;
                            let path = resolve_path(&std::env::current_dir()?, file);
                            let mut file =
                                OpenOptions::new().append(true).create(true).open(path)?;
                            file.write_all(content.as_bytes())?;
                            IDLValue::Text(content.to_string())
                        }
                        _ => return Err(anyhow!("wasm_profiling expects (file path, content)")),
                    },
                    "stringify" => {
                        use std::fmt::Write;
                        let mut res = String::new();
                        for arg in args {
                            write!(&mut res, "{}", crate::utils::stringify(&arg)?)?;
                        }
                        IDLValue::Text(res)
                    }
                    "concat" => match args.as_slice() {
                        [IDLValue::Vec(s1), IDLValue::Vec(s2)] => {
                            let mut res = Vec::from(s1.as_slice());
                            res.extend_from_slice(s2);
                            IDLValue::Vec(res)
                        }
                        [IDLValue::Text(s1), IDLValue::Text(s2)] => {
                            IDLValue::Text(String::from(s1) + s2)
                        }
                        [IDLValue::Record(f1), IDLValue::Record(f2)] => {
                            let mut fs = Vec::from(f1.as_slice());
                            fs.extend_from_slice(f2);
                            fs.sort_unstable_by_key(|IDLField { id, .. }| id.get_id());
                            check_unique(fs.iter().map(|f| &f.id))?;
                            IDLValue::Record(fs)
                        }
                        _ => return Err(anyhow!("concat expects two vec, record or text")),
                    },
                    "add" | "sub" | "mul" | "div" => match args.as_slice() {
                        [IDLValue::Float32(_) | IDLValue::Float64(_), _] | [_, IDLValue::Float32(_) | IDLValue::Float64(_)] => {
                            let IDLValue::Float64(v1) = cast_type(args[0].clone(), &TypeInner::Float64.into())? else { panic!() };
                            let IDLValue::Float64(v2) = cast_type(args[1].clone(), &TypeInner::Float64.into())? else { panic!() };
                            IDLValue::Float64(match func.as_str() {
                                "add" => v1 + v2,
                                "sub" => v1 - v2,
                                "mul" => v1 * v2,
                                "div" => v1 / v2,
                                _ => unreachable!(),
                            })
                        }
                        [v1, v2] => {
                            let IDLValue::Int(v1) = cast_type(v1.clone(), &TypeInner::Int.into())? else { panic!() };
                            let IDLValue::Int(v2) = cast_type(v2.clone(), &TypeInner::Int.into())? else { panic!() };
                            IDLValue::Number(match func.as_str() {
                                "add" => v1 + v2,
                                "sub" => v1 - v2,
                                "mul" => v1 * v2,
                                "div" => v1 / v2,
                                _ => unreachable!(),
                            }.to_string())
                        }
                        _ => return Err(anyhow!("add expects two numbers")),
                    }
                    func => match helper.func_env.0.get(func) {
                        None => return Err(anyhow!("Unknown function {}", func)),
                        Some((formal_args, body)) => {
                            if formal_args.len() != args.len() {
                                return Err(anyhow!(
                                    "{} expects {} arguments, but {} is provided",
                                    func,
                                    formal_args.len(),
                                    args.len()
                                ));
                            }
                            let mut helper = helper.spawn();
                            for (id, v) in formal_args.iter().zip(args.into_iter()) {
                                helper.env.0.insert(id.to_string(), v);
                            }
                            for cmd in body.iter() {
                                cmd.clone().run(&mut helper)?;
                            }
                            let res = helper.env.0.get("_").unwrap_or(&IDLValue::Null).clone();
                            res
                        }
                    },
                }
            }
            Exp::Decode { method, blob } => {
                let blob = blob.eval(helper)?;
                if *blob.value_ty() != TypeInner::Vec(TypeInner::Nat8.into()) {
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
                        let info = method.get_info(helper)?;
                        if let Some((env, func)) = info.signature {
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
                let opt_info = if let Some(method) = &method {
                    Some(method.get_info(helper)?)
                } else {
                    None
                };
                let bytes = if let Some(MethodInfo {
                    signature: Some((env, func)),
                    ..
                }) = &opt_info
                {
                    args.to_bytes_with_types(env, &func.args)?
                } else {
                    args.to_bytes()?
                };
                match mode {
                    CallMode::Encode => {
                        IDLValue::Vec(bytes.into_iter().map(IDLValue::Nat8).collect())
                    }
                    CallMode::Call => {
                        use crate::profiling::{get_cycles, ok_to_profile};
                        let method = method.unwrap(); // okay to unwrap from parser
                        let info = opt_info.unwrap();
                        let ok_to_profile = ok_to_profile(helper, &info);
                        let before_cost = if ok_to_profile {
                            get_cycles(&helper.agent, &info.canister_id)?
                        } else {
                            0
                        };
                        let res = call(
                            &helper.agent,
                            &info.canister_id,
                            &method.method,
                            &bytes,
                            &info.signature,
                            &helper.offline,
                        )?;
                        if ok_to_profile {
                            let cost = get_cycles(&helper.agent, &info.canister_id)? - before_cost;
                            println!("Cost: {cost} Wasm instructions");
                            let cost = IDLValue::Record(vec![IDLField {
                                id: Label::Named("__cost".to_string()),
                                val: IDLValue::Int64(cost),
                            }]);
                            let res = IDLArgs::new(&[args_to_value(res), cost]);
                            args_to_value(res)
                        } else {
                            args_to_value(res)
                        }
                    }
                    CallMode::Proxy(id) => {
                        let method = method.unwrap();
                        let canister_id = str_to_principal(&method.canister, helper)?;
                        let proxy_id = str_to_principal(&id, helper)?;
                        let mut env = MyHelper::new(
                            helper.agent.clone(),
                            helper.agent_url.clone(),
                            helper.offline.clone(),
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
                        let cmds = pretty_parse::<crate::command::Commands>("forward_call", &code)?;
                        for (cmd, _) in cmds.0.into_iter() {
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

impl std::str::FromStr for Exp {
    type Err = ParserError;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let lexer = Tokenizer::new(str);
        super::grammar::ExpParser::new().parse(lexer)
    }
}

#[derive(Debug)]
pub struct MethodInfo {
    pub canister_id: Principal,
    pub signature: Option<(TypeEnv, Function)>,
    pub profiling: Option<BTreeMap<u16, String>>,
}
impl Method {
    fn get_info(&self, helper: &MyHelper) -> Result<MethodInfo> {
        let canister_id = str_to_principal(&self.canister, helper)?;
        let agent = &helper.agent;
        let mut map = helper.canister_map.borrow_mut();
        Ok(match map.get(agent, &canister_id) {
            Err(_) => MethodInfo {
                canister_id,
                signature: None,
                profiling: None,
            },
            Ok(info) => {
                let signature = if self.method == "__init_args" {
                    Some((
                        info.env.clone(),
                        Function {
                            args: info.init.as_ref().unwrap_or(&Vec::new()).clone(),
                            rets: Vec::new(),
                            modes: Vec::new(),
                        },
                    ))
                } else {
                    info.methods
                        .get(&self.method)
                        .or_else(|| {
                            if !self.method.starts_with("__") {
                                eprintln!(
                                    "Warning: cannot get type for {}.{}, use types infered from textual value",
                                    self.canister, self.method
                                );
                            }
                            None
                        })
                        .map(|ty| (info.env.clone(), ty.clone()))
                };
                MethodInfo {
                    canister_id,
                    signature,
                    profiling: info.profiling.clone(),
                }
            }
        })
    }
}

#[tokio::main]
async fn call(
    agent: &Agent,
    canister_id: &Principal,
    method: &str,
    args: &[u8],
    opt_func: &Option<(TypeEnv, Function)>,
    offline: &Option<OfflineOutput>,
) -> anyhow::Result<IDLArgs> {
    use crate::offline::*;
    let effective_id = get_effective_canister_id(*canister_id, method, args)?;
    let is_query = opt_func
        .as_ref()
        .map(|(_, f)| f.is_query())
        .unwrap_or(false);
    let bytes = if is_query {
        let mut builder = agent.query(canister_id, method);
        builder = builder
            .with_arg(args)
            .with_effective_canister_id(effective_id);
        if let Some(offline) = offline {
            let signed = builder.sign()?;
            let message = Ingress {
                call_type: "query".to_owned(),
                request_id: None,
                content: hex::encode(signed.signed_query),
            };
            output_message(serde_json::to_string(&message)?, offline)?;
            return Ok(IDLArgs::new(&[]));
        } else {
            builder.call().await?
        }
    } else {
        let mut builder = agent.update(canister_id, method);
        builder = builder
            .with_arg(args)
            .with_effective_canister_id(effective_id);
        if let Some(offline) = offline {
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
            output_message(serde_json::to_string(&message)?, offline)?;
            return Ok(IDLArgs::new(&[]));
        } else {
            builder.call_and_wait().await?
        }
    };
    let res = if let Some((env, func)) = opt_func {
        IDLArgs::from_bytes_with_types(&bytes, env, &func.rets)?
    } else {
        IDLArgs::from_bytes(&bytes)?
    };
    Ok(res)
}
