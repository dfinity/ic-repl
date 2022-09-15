use crate::helper::OfflineOutput;
use candid::Principal;

#[derive(serde::Serialize)]
pub struct Ingress {
    pub call_type: String,
    pub request_id: Option<String>,
    pub content: String,
}
#[derive(serde::Serialize)]
pub struct RequestStatus {
    pub canister_id: Principal,
    pub request_id: String,
    pub content: String,
}
#[derive(serde::Serialize)]
pub struct IngressWithStatus {
    pub ingress: Ingress,
    pub request_status: RequestStatus,
}
static mut PNG_COUNTER: u32 = 0;
pub fn output_message(json: String, format: &OfflineOutput) -> anyhow::Result<()> {
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
