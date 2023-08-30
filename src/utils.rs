use crate::helper::MyHelper;
use anyhow::{anyhow, Context, Result};
use candid::bindings::candid::value::number_to_string;
use candid::parser::configs::Configs;
use candid::types::value::{IDLArgs, IDLField, IDLValue};
use candid::types::{Label, Type, TypeInner};
use candid::Principal;
use candid::TypeEnv;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub fn stringify(v: &IDLValue) -> anyhow::Result<Cow<str>> {
    Ok(match v {
        IDLValue::Text(str) => Cow::Borrowed(str),
        IDLValue::Number(_)
        | IDLValue::Int64(_)
        | IDLValue::Int32(_)
        | IDLValue::Int16(_)
        | IDLValue::Int8(_)
        | IDLValue::Int(_)
        | IDLValue::Nat64(_)
        | IDLValue::Nat32(_)
        | IDLValue::Nat16(_)
        | IDLValue::Nat8(_)
        | IDLValue::Nat(_)
        | IDLValue::Float32(_)
        | IDLValue::Float64(_) =>
        // Not using Debug print to omit type annotations
        {
            Cow::Owned(number_to_string(v))
        }
        IDLValue::Principal(id) => Cow::Owned(id.to_string()),
        IDLValue::Service(id) => Cow::Owned(id.to_string()),
        IDLValue::Func(id, meth) => Cow::Owned(format!("{id}.{meth}")),
        IDLValue::Null => Cow::Borrowed("null"),
        IDLValue::None => Cow::Borrowed("none"),
        IDLValue::Reserved => Cow::Borrowed("reserved"),
        _ => Cow::Owned(format!("{v:?}")), // TODO: need to remove type annotations for inner values
    })
}

fn num_cast_helper(v: IDLValue, truncate_float: bool) -> Result<String> {
    Ok(match v {
        IDLValue::Number(n) => n,
        IDLValue::Int64(n) => n.to_string(),
        IDLValue::Int32(n) => n.to_string(),
        IDLValue::Int16(n) => n.to_string(),
        IDLValue::Int8(n) => n.to_string(),
        IDLValue::Int(n) => n.to_string(),
        IDLValue::Nat64(n) => n.to_string(),
        IDLValue::Nat32(n) => n.to_string(),
        IDLValue::Nat16(n) => n.to_string(),
        IDLValue::Nat8(n) => n.to_string(),
        IDLValue::Nat(n) => n.to_string(),
        IDLValue::Float32(f) => if truncate_float { f.trunc() } else { f }.to_string(),
        IDLValue::Float64(f) => if truncate_float { f.trunc() } else { f }.to_string(),
        _ => return Err(anyhow!("{v} is not a number")),
    })
}

/// This function allows conversions between text and blob, principal and service/func, and all number types.
pub fn cast_type(v: IDLValue, ty: &Type) -> Result<IDLValue> {
    Ok(match (v, ty.as_ref()) {
        (_, TypeInner::Reserved) => IDLValue::Reserved,
        (IDLValue::Null, TypeInner::Null) => IDLValue::Null,
        (IDLValue::Bool(b), TypeInner::Bool) => IDLValue::Bool(b),
        (IDLValue::Null | IDLValue::Reserved | IDLValue::None, TypeInner::Opt(_)) => IDLValue::None,
        // No fallback to None for option
        (IDLValue::Opt(v), TypeInner::Opt(t)) => IDLValue::Opt(Box::new(cast_type(*v, t)?)),
        (IDLValue::Vec(vec), TypeInner::Vec(t)) => {
            let mut res = Vec::with_capacity(vec.len());
            for e in vec.into_iter() {
                let v = cast_type(e, t)?;
                res.push(v);
            }
            IDLValue::Vec(res)
        }
        // text <--> blob
        (IDLValue::Text(s), TypeInner::Text) => IDLValue::Text(s),
        (IDLValue::Vec(vec), TypeInner::Text)
            if vec.is_empty() || matches!(vec[0], IDLValue::Nat8(_)) =>
        {
            let bytes: Vec<_> = vec
                .into_iter()
                .map(|x| {
                    let IDLValue::Nat8(v) = x else {
                        unreachable!("not a blob")
                    };
                    v
                })
                .collect();
            IDLValue::Text(String::from_utf8(bytes)?)
        }
        (IDLValue::Text(str), TypeInner::Vec(t)) if matches!(t.as_ref(), TypeInner::Nat8) => {
            let blob = str.into_bytes().into_iter().map(IDLValue::Nat8).collect();
            IDLValue::Vec(blob)
        }
        // reference types
        (
            IDLValue::Principal(id) | IDLValue::Service(id) | IDLValue::Func(id, _),
            TypeInner::Principal,
        ) => IDLValue::Principal(id),
        (
            IDLValue::Principal(id) | IDLValue::Service(id) | IDLValue::Func(id, _),
            TypeInner::Service(_),
        ) => IDLValue::Service(id),
        (IDLValue::Func(id, meth), TypeInner::Func(_)) => IDLValue::Func(id, meth),
        // number types
        (v, TypeInner::Int) => IDLValue::Int(num_cast_helper(v, true)?.parse::<candid::Int>()?),
        (v, TypeInner::Nat) => IDLValue::Nat(num_cast_helper(v, true)?.parse::<candid::Nat>()?),
        (v, TypeInner::Nat8) => IDLValue::Nat8(num_cast_helper(v, true)?.parse::<u8>()?),
        (v, TypeInner::Nat16) => IDLValue::Nat16(num_cast_helper(v, true)?.parse::<u16>()?),
        (v, TypeInner::Nat32) => IDLValue::Nat32(num_cast_helper(v, true)?.parse::<u32>()?),
        (v, TypeInner::Nat64) => IDLValue::Nat64(num_cast_helper(v, true)?.parse::<u64>()?),
        (v, TypeInner::Int8) => IDLValue::Int8(num_cast_helper(v, true)?.parse::<i8>()?),
        (v, TypeInner::Int16) => IDLValue::Int16(num_cast_helper(v, true)?.parse::<i16>()?),
        (v, TypeInner::Int32) => IDLValue::Int32(num_cast_helper(v, true)?.parse::<i32>()?),
        (v, TypeInner::Int64) => IDLValue::Int64(num_cast_helper(v, true)?.parse::<i64>()?),
        (v, TypeInner::Float32) => IDLValue::Float32(num_cast_helper(v, false)?.parse::<f32>()?),
        (v, TypeInner::Float64) => IDLValue::Float64(num_cast_helper(v, false)?.parse::<f64>()?),
        // error
        (_, TypeInner::Record(_) | TypeInner::Variant(_)) => {
            return Err(anyhow!("{ty} annotation not implemented"))
        }
        (v, _) => return Err(anyhow!("Cannot cast {v} to type {ty}")),
    })
}

pub fn str_to_principal(id: &str, helper: &MyHelper) -> Result<Principal> {
    let try_id = Principal::from_text(id);
    Ok(match try_id {
        Ok(id) => id,
        Err(_) => match helper.env.0.get(id) {
            Some(IDLValue::Principal(id)) => *id,
            Some(IDLValue::Service(id)) => *id,
            Some(IDLValue::Func(id, _)) => *id,
            _ => return Err(anyhow!("{} is not a canister id", id)),
        },
    })
}

pub fn get_effective_canister_id(
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

pub fn args_to_value(mut args: IDLArgs) -> IDLValue {
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

pub fn random_value(env: &TypeEnv, ty: &Type, config: &Configs) -> candid::Result<String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let seed: Vec<_> = (0..2048).map(|_| rng.gen::<u8>()).collect();
    let result = IDLArgs::any(&seed, config, env, &[ty.clone()])?;
    Ok(result.args[0].to_string())
}

pub fn resolve_path(base: &Path, file: &str) -> PathBuf {
    let file = PathBuf::from(shellexpand::tilde(file).into_owned());
    if file.is_absolute() {
        file
    } else {
        base.join(file)
    }
}

pub fn get_dfx_hsm_pin() -> Result<String, String> {
    std::env::var("DFX_HSM_PIN").or_else(|_| {
        rpassword::prompt_password("HSM PIN: ")
            .context("No DFX_HSM_PIN environment variable and cannot read HSM PIN from tty")
            .map_err(|e| e.to_string())
    })
}
