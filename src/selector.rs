use super::exp::Exp;
use super::helper::MyHelper;
use anyhow::{anyhow, Result};
use candid::{
    parser::value::{IDLValue, VariantValue},
    types::Label,
};

#[derive(Debug, Clone)]
pub enum Selector {
    Index(u64),
    Field(String),
    Option,
    Map(String),
    Filter(String),
    Fold(Exp, String),
}
impl Selector {
    fn to_label(&self) -> Label {
        match self {
            Selector::Index(idx) => Label::Id(*idx as u32),
            Selector::Field(name) => Label::Named(name.to_string()),
            _ => unreachable!(),
        }
    }
}
pub fn project(helper: &MyHelper, value: IDLValue, path: &[Selector]) -> Result<IDLValue> {
    let mut result = value;
    for head in path.iter() {
        match (result, head) {
            (IDLValue::Opt(opt), Selector::Option) => result = *opt,
            (IDLValue::Vec(mut vs), Selector::Index(idx)) => {
                let idx = *idx as usize;
                if idx < vs.len() {
                    result = vs.swap_remove(idx);
                } else {
                    return Err(anyhow!("{} out of bound {}", idx, vs.len()));
                }
            }
            (IDLValue::Vec(vs), head @ (Selector::Map(func) | Selector::Filter(func))) => {
                let mut new_helper = helper.spawn();
                let mut res = Vec::with_capacity(vs.len());
                for v in vs.into_iter() {
                    new_helper.env.0.insert(String::new(), v.clone());
                    let arg = Exp::Path(String::new(), Vec::new());
                    let exp = Exp::Apply(func.to_string(), vec![arg]);
                    match (head, exp.eval(&new_helper)) {
                        (Selector::Map(_), v) => res.push(v?),
                        (Selector::Filter(_), Ok(IDLValue::Bool(false))) => (),
                        (Selector::Filter(_), Ok(_)) => res.push(v),
                        (Selector::Filter(_), Err(_)) => (),
                        (_, _) => unreachable!(),
                    }
                }
                result = IDLValue::Vec(res);
            }
            (IDLValue::Vec(vs), Selector::Fold(init, func)) => {
                let init = init.clone().eval(helper)?;
                let mut new_helper = helper.spawn();
                let mut acc = init;
                for v in vs.into_iter() {
                    new_helper.env.0.insert(String::new(), v);
                    let arg = Exp::Path(String::new(), Vec::new());
                    new_helper.env.0.insert("_".to_string(), acc.clone());
                    let accu = Exp::Path("_".to_string(), Vec::new());
                    let exp = Exp::Apply(func.to_string(), vec![accu, arg]);
                    acc = exp.eval(&new_helper)?;
                }
                result = acc;
            }
            (IDLValue::Record(fs), field @ (Selector::Index(_) | Selector::Field(_))) => {
                let id = field.to_label();
                if let Some(v) = fs.into_iter().find(|f| f.id == id) {
                    result = v.val;
                } else {
                    return Err(anyhow!("record field {:?} not found", field));
                }
            }
            (
                IDLValue::Variant(VariantValue(f, _)),
                field @ (Selector::Index(_) | Selector::Field(_)),
            ) => {
                if field.to_label() == f.id {
                    result = f.val;
                } else {
                    return Err(anyhow!("variant field {:?} not found", field));
                }
            }
            (value, head) => {
                return Err(anyhow!(
                    "selector {:?} cannot be applied to {}",
                    head,
                    value
                ))
            }
        }
    }
    Ok(result)
}
