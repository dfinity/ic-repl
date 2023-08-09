use crate::helper::{MyHelper, OfflineOutput};
use anyhow::{anyhow, Context, Result};
use candid::Principal;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Ingress {
    pub call_type: String,
    pub request_id: Option<String>,
    pub content: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct RequestStatus {
    pub canister_id: Principal,
    pub request_id: String,
    pub content: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct IngressWithStatus {
    pub ingress: Ingress,
    pub request_status: Option<RequestStatus>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Messages {
    pub replica_url: Option<String>,
    pub messages: Vec<IngressWithStatus>,
}

static mut PNG_COUNTER: u32 = 0;

impl Ingress {
    pub fn parse(&self) -> Result<(Principal, Principal, String, Vec<u8>)> {
        use serde_cbor::Value;
        let cbor: Value = serde_cbor::from_slice(&hex::decode(&self.content)?)
            .context("Invalid cbor data in the content of the message.")?;
        if let Value::Map(m) = cbor {
            let cbor_content = m
                .get(&Value::Text("content".to_string()))
                .ok_or_else(|| anyhow!("Invalid cbor content"))?;
            if let Value::Map(m) = cbor_content {
                if let (
                    Some(Value::Bytes(sender)),
                    Some(Value::Bytes(canister_id)),
                    Some(Value::Text(method_name)),
                    Some(Value::Bytes(arg)),
                ) = (
                    m.get(&Value::Text("sender".to_string())),
                    m.get(&Value::Text("canister_id".to_string())),
                    m.get(&Value::Text("method_name".to_string())),
                    m.get(&Value::Text("arg".to_string())),
                ) {
                    let sender = Principal::try_from(sender)?;
                    let canister_id = Principal::try_from(canister_id)?;
                    return Ok((sender, canister_id, method_name.to_string(), arg.to_vec()));
                }
            }
        }
        Err(anyhow!("Invalid cbor content"))
    }
}
pub fn output_message(json: String, format: &OfflineOutput) -> Result<()> {
    match format {
        OfflineOutput::Json => println!("{json}"),
        _ => {
            use base64::{
                engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
                Engine,
            };
            use libflate::gzip;
            use qrcode::{render::unicode, QrCode};
            use std::io::Write;
            eprintln!("json length: {}", json.len());
            let mut encoder = gzip::Encoder::new(Vec::new())?;
            encoder.write_all(json.as_bytes())?;
            let zipped = encoder.finish().into_result()?;
            let engine = if matches!(format, OfflineOutput::PngNoUrl | OfflineOutput::AsciiNoUrl) {
                STANDARD_NO_PAD
            } else {
                URL_SAFE_NO_PAD
            };
            let base64 = engine.encode(zipped);
            eprintln!("base64 length: {}", base64.len());
            let msg = match format {
                OfflineOutput::Ascii(url) | OfflineOutput::Png(url) => url.to_owned() + &base64,
                _ => base64,
            };
            let code = QrCode::new(msg)?;
            match format {
                OfflineOutput::Ascii(_) | OfflineOutput::AsciiNoUrl => {
                    let img = code.render::<unicode::Dense1x2>().build();
                    println!("{img}");
                }
                OfflineOutput::Png(_) | OfflineOutput::PngNoUrl => {
                    let img = code.render::<image::Luma<u8>>().build();
                    let filename = unsafe {
                        PNG_COUNTER += 1;
                        format!("msg{PNG_COUNTER}.png")
                    };
                    img.save(&filename)?;
                    println!("QR code saved to {filename}");
                }
                _ => unreachable!(),
            }
        }
    };
    Ok(())
}
pub fn dump_ingress(msgs: &[IngressWithStatus], replica_url: String) -> Result<()> {
    use std::fs::File;
    use std::io::Write;
    let messages = msgs.to_vec();
    let msgs = Messages {
        messages,
        replica_url: Some(replica_url),
    };
    let json = serde_json::to_string(&msgs)?;
    let mut file = File::create("messages.json")?;
    file.write_all(json.as_bytes())?;
    Ok(())
}
#[tokio::main]
pub async fn send_messages(helper: MyHelper, msgs: &Messages) -> Result<()> {
    let len = msgs.messages.len();
    println!("Sending {} messages to {}", len, helper.agent_url);
    for (i, msg) in msgs.messages.iter().enumerate() {
        print!("[{}/{}] ", i + 1, len);
        send(&helper, &msg.ingress).await?;
    }
    Ok(())
}
async fn send(helper: &MyHelper, message: &Ingress) -> Result<()> {
    use crate::exp::Method;
    use candid::IDLArgs;
    let (sender, canister_id, method_name, bytes) = message.parse()?;
    let meth = Method {
        canister: canister_id.to_string(),
        method: method_name.clone(),
    };
    let opt_func = meth.get_info(helper)?.signature;
    let args = if let Some((env, func)) = opt_func {
        IDLArgs::from_bytes_with_types(&bytes, &env, &func.args)?
    } else {
        IDLArgs::from_bytes(&bytes)?
    };

    println!("Sending {} call as {}:", message.call_type, sender);
    println!("  call \"{}\".{}{};", canister_id, method_name, args);
    println!("Do you want to send this message? [y/N]");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !["y", "yes"].contains(&input.to_lowercase().trim()) {
        std::process::exit(0);
    }

    Ok(())
}
