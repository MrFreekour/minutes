use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use std::path::PathBuf;

fn required_arg(name: &str) -> PathBuf {
    std::env::args_os()
        .nth(match name {
            "archive" => 1,
            "signature" => 2,
            _ => unreachable!(),
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: verify_updater_signature <archive> <signature>");
            std::process::exit(2);
        })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = required_arg("archive");
    let signature_path = required_arg("signature");
    let config: serde_json::Value = serde_json::from_slice(include_bytes!("../tauri.conf.json"))?;
    let encoded_key = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or("Archive updater public key is missing from tauri.conf.json")?;
    let key_text =
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded_key)?)?;
    let signature_text = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(std::fs::read_to_string(signature_path)?.trim())?,
    )?;

    let public_key = PublicKey::decode(&key_text)?;
    let signature = Signature::decode(&signature_text)?;
    let archive = std::fs::read(archive_path)?;
    public_key.verify(&archive, &signature, true)?;
    println!("updater_signature=valid");
    Ok(())
}
