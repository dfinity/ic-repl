use crate::helper::MyHelper;
use anyhow::{anyhow, Context, Result};
use candid::pretty::candid::value::number_to_string;
use candid::types::value::{IDLArgs, IDLField, IDLValue};
use candid::types::{Label, Type, TypeInner};
use candid::{Principal, TypeEnv};
use candid_parser::configs::Configs;
use ic_agent::Agent;
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
        IDLValue::Number(n) => n.replace('_', ""),
        IDLValue::Int64(n) => n.to_string(),
        IDLValue::Int32(n) => n.to_string(),
        IDLValue::Int16(n) => n.to_string(),
        IDLValue::Int8(n) => n.to_string(),
        IDLValue::Int(n) => n.to_string().replace('_', ""),
        IDLValue::Nat64(n) => n.to_string(),
        IDLValue::Nat32(n) => n.to_string(),
        IDLValue::Nat16(n) => n.to_string(),
        IDLValue::Nat8(n) => n.to_string(),
        IDLValue::Nat(n) => n.to_string().replace('_', ""),
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
            if matches!(t.as_ref(), TypeInner::Nat8) {
                let blob = res
                    .into_iter()
                    .filter_map(|v| match v {
                        IDLValue::Nat8(n) => Some(n),
                        _ => None,
                    })
                    .collect();
                IDLValue::Blob(blob)
            } else {
                IDLValue::Vec(res)
            }
        }
        (IDLValue::Blob(blob), TypeInner::Vec(t)) => {
            let mut res = Vec::with_capacity(blob.len());
            for e in blob.into_iter() {
                let v = cast_type(IDLValue::Nat8(e), t)?;
                res.push(v);
            }
            IDLValue::Vec(res)
        }
        // text <--> blob
        (IDLValue::Text(s), TypeInner::Text) => IDLValue::Text(s),
        (IDLValue::Blob(b), TypeInner::Text) => IDLValue::Text(String::from_utf8(b)?),
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
            IDLValue::Blob(str.into_bytes())
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
) -> anyhow::Result<Option<Principal>> {
    use candid::{CandidType, Decode, Deserialize};
    if canister_id == Principal::management_canister() {
        match method {
            "create_canister" | "raw_rand" => Err(anyhow!(
                "{} can only be called via inter-canister call.",
                method
            )),
            "provisional_create_canister_with_cycles" => Ok(None),
            "install_chunked_code" => {
                #[derive(CandidType, Deserialize)]
                struct Arg {
                    target_canister: Principal,
                }
                let args = Decode!(args, Arg).map_err(|_| {
                    anyhow!("{} can only be called via inter-canister call.", method)
                })?;
                Ok(Some(args.target_canister))
            }
            _ => {
                #[derive(CandidType, Deserialize)]
                struct Arg {
                    canister_id: Principal,
                }
                let args = Decode!(args, Arg).map_err(|_| {
                    anyhow!("{} can only be called via inter-canister call.", method)
                })?;
                Ok(Some(args.canister_id))
            }
        }
    } else {
        Ok(Some(canister_id))
    }
}

pub fn as_u32(v: &IDLValue) -> Result<u32> {
    match v {
        IDLValue::Number(n) => {
            let n = n.parse::<u32>()?;
            Ok(n)
        }
        IDLValue::Nat32(n) => Ok(*n),
        _ => Err(anyhow!("{v} is not a number")),
    }
}

pub fn get_field<'a>(fs: &'a [IDLField], key: &'a str) -> Option<&'a IDLValue> {
    fs.iter()
        .find(|f| f.id == Label::Named(key.to_string()))
        .map(|f| &f.val)
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
    ty: &Type,
    config: Configs,
    scope: candid_parser::configs::Scope,
) -> candid_parser::Result<String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let seed: Vec<_> = (0..2048).map(|_| rng.gen::<u8>()).collect();
    let result = candid_parser::random::any(&seed, config, env, &[ty.clone()], &Some(scope))?;
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

#[tokio::main]
pub async fn fetch_state_path(agent: &Agent, mut path: StatePath) -> anyhow::Result<IDLValue> {
    if path.effective_id.is_none() {
        let id = if path.path.len() >= 3
            && path.path[0] == "subnet".into()
            && matches!(path.kind, StateKind::Canister)
        {
            get_canister_id_from_subnet(agent, path.path[1].clone()).await.ok_or_else(|| anyhow!("Cannot find any canister on this subnet id. Put the effective canister id as the first argument"))?
        } else {
            Principal::from_text(match path.kind {
                StateKind::Canister => "ryjl3-tyaaa-aaaaa-aaaba-cai",
                StateKind::Subnet => {
                    "tdb26-jop6k-aogll-7ltgs-eruif-6kk7m-qpktf-gdiqx-mxtrf-vb5e6-eqe"
                }
            })?
        };
        path.effective_id = Some(id);
        eprintln!("Using {} as effective canister/subnet id. To change it, put the effective id as the first argument.", id);
    }
    fetch_state_path_(agent, path).await
}
pub async fn fetch_metadata(
    agent: &Agent,
    id: Principal,
    sub_paths: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut path: Vec<ic_agent::hash_tree::Label<Vec<u8>>> =
        vec!["canister".as_bytes().into(), id.as_slice().into()];
    path.extend(sub_paths.split('/').map(|s| s.as_bytes().into()));
    let path = StatePath {
        path,
        effective_id: Some(id),
        kind: StateKind::Canister,
        result: StateType::Blob,
    };
    match fetch_state_path_(agent, path).await? {
        IDLValue::Blob(b) => Ok(b),
        _ => unreachable!(),
    }
}
async fn get_canister_id_from_subnet(
    agent: &Agent,
    subnet_id: ic_agent::hash_tree::Label<Vec<u8>>,
) -> Option<Principal> {
    let effective_id = Principal::from_slice(subnet_id.as_bytes());
    let path = StatePath {
        path: vec!["subnet".into(), subnet_id, "canister_ranges".into()],
        effective_id: Some(effective_id),
        kind: StateKind::Subnet,
        result: StateType::Blob,
    };
    let bytes = match fetch_state_path_(agent, path).await.ok()? {
        IDLValue::Blob(b) => b,
        _ => unreachable!(),
    };
    let res = serde_cbor::from_slice::<Vec<(Principal, Principal)>>(&bytes).ok()?;
    res.first().map(|(a, _)| *a)
}
async fn fetch_state_path_(agent: &Agent, path: StatePath) -> anyhow::Result<IDLValue> {
    use ic_agent::{hash_tree::SubtreeLookupResult, lookup_value};
    let effective_id = path.effective_id.unwrap();
    let cert = match path.kind {
        StateKind::Subnet => {
            agent
                .read_subnet_state_raw(vec![path.path.clone()], effective_id)
                .await?
        }
        StateKind::Canister => {
            agent
                .read_state_raw(vec![path.path.clone()], effective_id)
                .await?
        }
    };
    if matches!(path.result, StateType::Subtree) {
        let tree = match cert.tree.lookup_subtree(&path.path) {
            SubtreeLookupResult::Found(t) => t,
            SubtreeLookupResult::Absent => return Err(anyhow!("Subtree absent")),
            SubtreeLookupResult::Unknown => return Err(anyhow!("Subtree unknown")),
        };
        let paths = tree.list_paths();
        let ids: std::collections::HashSet<_> = paths
            .iter()
            .map(|p| Principal::from_slice(p[0].as_bytes()))
            .collect();
        Ok(IDLValue::Vec(
            ids.into_iter().map(IDLValue::Principal).collect(),
        ))
    } else {
        let bytes = lookup_value(&cert, path.path).map(<[u8]>::to_vec)?;
        Ok(match path.result {
            StateType::Blob => IDLValue::Blob(bytes),
            StateType::Text => IDLValue::Text(String::from_utf8(bytes)?),
            StateType::Nat => {
                let mut reader = std::io::Cursor::new(bytes);
                let n = candid::Nat::decode(&mut reader)?;
                IDLValue::Nat(n)
            }
            StateType::Controllers => {
                let res = serde_cbor::from_slice::<Vec<Principal>>(&bytes)?;
                IDLValue::Vec(res.into_iter().map(IDLValue::Principal).collect())
            }
            StateType::Ranges => {
                let res = serde_cbor::from_slice::<Vec<(Principal, Principal)>>(&bytes)?;
                IDLValue::Vec(
                    res.into_iter()
                        .map(|(a, b)| {
                            IDLValue::Record(vec![
                                IDLField {
                                    id: Label::Id(0),
                                    val: IDLValue::Principal(a),
                                },
                                IDLField {
                                    id: Label::Id(1),
                                    val: IDLValue::Principal(b),
                                },
                            ])
                        })
                        .collect(),
                )
            }
            StateType::Metrics => {
                let res = serde_cbor::from_slice::<ic_transport_types::SubnetMetrics>(&bytes)?;
                let cycles = res.consumed_cycles_total as u64;
                let cycles_deleted = (res.consumed_cycles_total >> 64) as u64;
                IDLValue::Record(vec![
                    IDLField {
                        id: Label::Named("num_canisters".to_string()),
                        val: IDLValue::Nat64(res.num_canisters),
                    },
                    IDLField {
                        id: Label::Named("canister_state_bytes".to_string()),
                        val: IDLValue::Nat64(res.canister_state_bytes),
                    },
                    IDLField {
                        id: Label::Named("consumed_cycles_total".to_string()),
                        val: IDLValue::Nat64(cycles),
                    },
                    IDLField {
                        id: Label::Named("consumed_cycles_total_deleted".to_string()),
                        val: IDLValue::Nat64(cycles_deleted),
                    },
                    IDLField {
                        id: Label::Named("update_transactions_total".to_string()),
                        val: IDLValue::Nat64(res.update_transactions_total),
                    },
                ])
            }
            StateType::Subtree => unreachable!(),
        })
    }
}
pub enum StateKind {
    Subnet,
    Canister,
}
pub enum StateType {
    Blob,
    Text,
    Nat,
    Controllers,
    Ranges,
    Metrics,
    Subtree,
}
pub struct StatePath {
    path: Vec<ic_agent::hash_tree::Label<Vec<u8>>>,
    pub effective_id: Option<Principal>,
    kind: StateKind,
    result: StateType,
}
pub fn parse_state_path(paths: &[IDLValue]) -> anyhow::Result<StatePath> {
    let mut res = Vec::new();
    let mut prefix = String::new();
    let mut kind = StateKind::Canister;
    let mut effective_id = None;
    let mut result = StateType::Blob;
    if paths.len() > 5 || paths.is_empty() {
        return Err(anyhow!("state path can only be 1-5 segments"));
    }
    for (i, v) in paths.iter().enumerate() {
        match v {
            IDLValue::Text(t) => {
                match i {
                    0 => {
                        prefix.clone_from(t);
                        if prefix == "subnet" {
                            kind = StateKind::Subnet;
                        }
                    }
                    1 => return Err(anyhow!("second path has to be a principal")),
                    2 => {
                        result = match (prefix.as_str(), t.as_str()) {
                            ("canister", "controllers") => StateType::Controllers,
                            (
                                "canister",
                                "metadata/candid:service"
                                | "metadata/candid:args"
                                | "metadata/motoko:stable-types",
                            ) => StateType::Text,
                            ("api_boundary_nodes", "domain" | "ipv4_address" | "ipv6_address") => {
                                StateType::Text
                            }
                            ("subnet", "canister_ranges") => StateType::Ranges,
                            ("subnet", "metrics") => StateType::Metrics,
                            ("subnet", "node") => {
                                // For some reason, /subnet/.../node is only available on canister read_state
                                effective_id = None;
                                kind = StateKind::Canister;
                                StateType::Subtree
                            }
                            _ => StateType::Blob,
                        };
                    }
                    _ => (),
                }
                res.extend(t.split('/').map(|str| str.as_bytes().into()));
            }
            IDLValue::Principal(id) => {
                match i {
                    1 => effective_id = Some(*id),
                    3 => result = StateType::Blob,
                    _ => return Err(anyhow!("{i}th path cannot be a principal")),
                }
                res.push(id.as_slice().into())
            }
            _ => return Err(anyhow!("state path can only be either text or principal")),
        }
    }
    if paths.len() == 1 {
        result = match prefix.as_str() {
            "time" => StateType::Nat,
            "subnet" | "api_boundary_nodes" => StateType::Subtree,
            _ => result,
        };
    }
    Ok(StatePath {
        path: res,
        effective_id,
        kind,
        result,
    })
}

#[test]
fn test_cast_type_big_num() {
    use candid::{Int, Nat};

    // cast to Nat64
    assert!(
        matches!(cast_type(IDLValue::Number("1_000_000".to_string()), &TypeInner::Nat64.into()),
               Ok(v) if v == IDLValue::Nat64(1_000_000u64))
    );
    assert!(
        matches!(cast_type(IDLValue::Nat(Nat::from(1_000_000u64)), &TypeInner::Nat64.into()),
               Ok(v) if v == IDLValue::Nat64(1_000_000u64))
    );
    assert!(
        matches!(cast_type(IDLValue::Int(Int::from(1_000_000i64)), &TypeInner::Nat64.into()),
               Ok(v) if v == IDLValue::Nat64(1_000_000u64))
    );

    fn pow<T: std::ops::MulAssign + From<u32> + Clone>(base: T, n: i32) -> T {
        let mut num = T::from(1u32);
        for _ in 0..n {
            num *= base.clone();
        }
        num
    }

    // cast to Float64
    for n in [0i32, 1, 10, 55] {
        assert!(
            matches!(cast_type(IDLValue::Number(pow(Nat::from(10u64), n).to_string()), &TypeInner::Float64.into()),
               Ok(v) if v == IDLValue::Float64(10f64.powi(n)))
        );
        assert!(
            matches!(cast_type(IDLValue::Nat(pow(Nat::from(10u64), n)), &TypeInner::Float64.into()),
               Ok(v) if v == IDLValue::Float64(10f64.powi(n)))
        );
        assert!(
            matches!(cast_type(IDLValue::Int(pow(Int::from(-10i64), n)), &TypeInner::Float64.into()),
               Ok(v) if v == IDLValue::Float64((-10f64).powi(n)))
        );
    }
}
