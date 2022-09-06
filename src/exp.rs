use super::command::resolve_path;
use super::helper::{MyHelper, OfflineOutput};
use super::token::{ParserError, Tokenizer};
use anyhow::{anyhow, Context, Result};
use candid::{
    parser::value::{IDLArgs, IDLField, IDLValue, VariantValue},
    types::{Function, Label, Type},
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
            Exp::AnnVal(v, ty) => {
                let arg = v.eval(helper)?;
                let env = TypeEnv::new();
                arg.annotate_type(true, &env, &ty)?
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
                    "file" => match args.as_slice() {
                        [IDLValue::Text(file)] => {
                            let path = resolve_path(&helper.base_path, file);
                            let blob: Vec<IDLValue> = std::fs::read(&path)
                                .with_context(|| format!("Cannot read {:?}", path))?
                                .into_iter()
                                .map(IDLValue::Nat8)
                                .collect();
                            IDLValue::Vec(blob)
                        }
                        _ => return Err(anyhow!("file expects file path")),
                    },
                    "wasm_profiling" => match args.as_slice() {
                        [IDLValue::Text(file)] => {
                            let path = resolve_path(&helper.base_path, file);
                            let blob = std::fs::read(&path)
                                .with_context(|| format!("Cannot read {:?}", path))?;
                            let mut m = walrus::Module::from_buffer(&blob)?;
                            ic_wasm::instrumentation::instrument(&mut m);
                            IDLValue::Vec(m.emit_wasm().into_iter().map(IDLValue::Nat8).collect())
                        }
                        _ => return Err(anyhow!("wasm_profiling expects file path")),
                    },
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
                        let method = method.unwrap(); // okay to unwrap from parser
                        let info = opt_info.unwrap();
                        let ok_to_profile = ok_to_profile(helper, &info);
                        let mut before_cost = 0;
                        if ok_to_profile.is_some() {
                            before_cost = get_cycles(&helper.agent, &info.canister_id)?;
                        }
                        let res = call(
                            &helper.agent,
                            &info.canister_id,
                            &method.method,
                            &bytes,
                            &info.signature,
                            &helper.offline,
                        )?;
                        if let Some(names) = ok_to_profile {
                            let after_cost = get_cycles(&helper.agent, &info.canister_id)?;
                            println!("Cost: {} Wasm instructions", after_cost - before_cost);
                            let title = format!("{}.{}", method.canister, method.method);
                            get_profiling(&helper.agent, &info.canister_id, names, title)?;
                        }
                        args_to_value(res)
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
                        let cmds =
                            crate::pretty_parse::<crate::command::Commands>("forward_call", &code)?;
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

pub fn project<'a>(value: &'a IDLValue, path: &[Selector]) -> Result<&'a IDLValue> {
    if path.is_empty() {
        return Ok(value);
    }
    let (head, tail) = (&path[0], &path[1..]);
    match (value, head) {
        (IDLValue::Opt(opt), Selector::Field(f)) if f == "?" => return project(opt, tail),
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
    Err(anyhow!("{:?} cannot be applied to {}", head, value))
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

#[derive(Debug)]
struct MethodInfo {
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
                            eprintln!(
                                "Warning: cannot get type for {}.{}, use types infered from textual value",
                                self.canister, self.method
                            );
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
static mut PNG_COUNTER: u32 = 0;
fn output_message(json: String, format: &OfflineOutput) -> anyhow::Result<()> {
    match format {
        OfflineOutput::Json => println!("{}", json),
        _ => {
            use libflate::gzip;
            use qrcode::{render::unicode, QrCode};
            use std::io::Write;
            eprintln!("json length: {}", json.len());
            let mut encoder = gzip::Encoder::new(Vec::new())?;
            encoder.write_all(json.as_bytes())?;
            let zipped = encoder.finish().into_result()?;
            let config = if matches!(format, OfflineOutput::PngNoUrl | OfflineOutput::AsciiNoUrl) {
                base64::STANDARD_NO_PAD
            } else {
                base64::URL_SAFE_NO_PAD
            };
            let base64 = base64::encode_config(&zipped, config);
            eprintln!("base64 length: {}", base64.len());
            let msg = match format {
                OfflineOutput::Ascii(url) | OfflineOutput::Png(url) => url.to_owned() + &base64,
                _ => base64,
            };
            let code = QrCode::new(&msg)?;
            match format {
                OfflineOutput::Ascii(_) | OfflineOutput::AsciiNoUrl => {
                    let img = code.render::<unicode::Dense1x2>().build();
                    println!("{}", img);
                    pause()?;
                }
                OfflineOutput::Png(_) | OfflineOutput::PngNoUrl => {
                    let img = code.render::<image::Luma<u8>>().build();
                    let filename = unsafe {
                        PNG_COUNTER += 1;
                        format!("msg{}.png", PNG_COUNTER)
                    };
                    img.save(&filename)?;
                    println!("QR code saved to {}", filename);
                }
                _ => unreachable!(),
            }
        }
    };
    Ok(())
}

fn pause() -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    eprint!("Press [enter] to continue...");
    stdout.flush()?;
    let _ = stdin.read(&mut [0u8])?;
    Ok(())
}

fn ok_to_profile<'a>(
    helper: &'a MyHelper,
    info: &'a MethodInfo,
) -> Option<&'a BTreeMap<u16, String>> {
    if helper.offline.is_none() {
        let names = info.profiling.as_ref()?;
        if info.signature.as_ref()?.1.is_query() {
            None
        } else {
            Some(names)
        }
    } else {
        None
    }
}

#[tokio::main]
async fn get_cycles(agent: &Agent, canister_id: &Principal) -> anyhow::Result<i64> {
    use candid::{Decode, Encode};
    let mut builder = agent.query(canister_id, "__get_cycles");
    let bytes = builder
        .with_arg(Encode!()?)
        .with_effective_canister_id(*canister_id)
        .call()
        .await?;
    Ok(Decode!(&bytes, i64)?)
}

#[tokio::main]
async fn get_profiling(
    agent: &Agent,
    canister_id: &Principal,
    names: &BTreeMap<u16, String>,
    title: String,
) -> anyhow::Result<()> {
    use candid::{Decode, Encode};
    let mut builder = agent.query(canister_id, "__get_profiling");
    let bytes = builder
        .with_arg(Encode!()?)
        .with_effective_canister_id(*canister_id)
        .call()
        .await?;
    let pairs = Decode!(&bytes, Vec<(i32, i64)>)?;
    if !pairs.is_empty() {
        render_profiling(pairs, names, title)?;
    }
    Ok(())
}

static mut SVG_COUNTER: u32 = 0;
fn render_profiling(
    input: Vec<(i32, i64)>,
    names: &BTreeMap<u16, String>,
    title: String,
) -> anyhow::Result<()> {
    use inferno::flamegraph::{from_reader, Options};
    use std::fmt::Write;
    let mut stack = Vec::new();
    let mut prefix = Vec::new();
    let mut result = String::new();
    let mut _total = 0;
    for (id, count) in input.into_iter() {
        if id >= 0 {
            stack.push((id, count, 0));
            let name = match names.get(&(id as u16)) {
                Some(name) => name.clone(),
                None => "func_".to_string() + &id.to_string(),
            };
            prefix.push(name);
        } else {
            match stack.pop() {
                None => return Err(anyhow!("pop empty stack")),
                Some((start_id, start, children)) => {
                    if start_id != -id {
                        return Err(anyhow!("func id mismatch"));
                    }
                    let cost = count - start;
                    let frame = prefix.join(";");
                    prefix.pop().unwrap();
                    if let Some((parent, parent_cost, children_cost)) = stack.pop() {
                        stack.push((parent, parent_cost, children_cost + cost));
                    } else {
                        _total += cost;
                    }
                    //println!("{} {}", frame, cost - children);
                    writeln!(&mut result, "{} {}", frame, cost - children)?;
                }
            }
        }
    }
    if !stack.is_empty() {
        eprintln!("A trap occured or trace is too large");
    }
    //println!("Cost: {} Wasm instructions", total);
    let mut opt = Options::default();
    opt.count_name = "instructions".to_string();
    opt.title = title;
    opt.image_width = Some(1024);
    opt.flame_chart = true;
    opt.no_sort = true;
    let reader = std::io::Cursor::new(result);
    let filename = unsafe {
        SVG_COUNTER += 1;
        format!("graph_{}.svg", SVG_COUNTER)
    };
    println!("Flamegraph written to {}", filename);
    let mut writer = std::fs::File::create(&filename)?;
    from_reader(&mut opt, reader, &mut writer)?;
    Ok(())
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
        builder
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
                let args = Decode!(args, Arg).map_err(|_| {
                    anyhow!("{} can only be called via inter-canister call.", method)
                })?;
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
