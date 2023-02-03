use crate::helper::MyHelper;
use anyhow::{anyhow, Context, Result};
use candid::bindings::candid::value::number_to_string;
use candid::parser::configs::Configs;
use candid::types::value::{IDLArgs, IDLField, IDLValue};
use candid::types::{Label, Type};
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
        | IDLValue::Float64(_) => Cow::Owned(number_to_string(v)),
        IDLValue::Principal(id) => Cow::Owned(id.to_string()),
        IDLValue::Bool(b) => Cow::Owned(b.to_string()),
        _ => return Err(anyhow!("Cannot stringify {}", v)),
    })
}

pub fn to_int(v: &IDLValue) -> Result<candid::Int> {
    Ok(match v {
        IDLValue::Number(n) => n.parse::<candid::Int>()?,
        IDLValue::Int(n) => n.clone(),
        IDLValue::Nat(n) => n.clone().into(),
        IDLValue::Nat8(n) => (*n).into(),
        IDLValue::Nat16(n) => (*n).into(),
        IDLValue::Nat32(n) => (*n).into(),
        IDLValue::Nat64(n) => (*n).into(),
        IDLValue::Int8(n) => (*n).into(),
        IDLValue::Int16(n) => (*n).into(),
        IDLValue::Int32(n) => (*n).into(),
        IDLValue::Int64(n) => (*n).into(),
        _ => return Err(anyhow!("Cannot convert {} to a number", v)),
    })
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

pub fn random_value(
    env: &TypeEnv,
    tys: &[Type],
    given_args: usize,
    config: &Configs,
) -> candid::Result<String> {
    use rand::Rng;
    use std::fmt::Write;
    let mut rng = rand::thread_rng();
    let seed: Vec<_> = (0..2048).map(|_| rng.gen::<u8>()).collect();
    let result = IDLArgs::any(&seed, config, env, tys)?;
    Ok(if given_args > 0 {
        if given_args <= tys.len() {
            let mut res = String::new();
            for v in result.args[given_args..].iter() {
                write!(&mut res, ", {v}").map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            res.push(')');
            res
        } else {
            "".to_owned()
        }
    } else {
        format!("{result}")
    })
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
