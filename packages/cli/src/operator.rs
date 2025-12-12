use crate::crypto::Keypair;
use anyhow::Result;

/// Generate a new operator key
pub fn generate() -> Result<()> {
    let keypair = Keypair::generate();
    let key_bytes = keypair.to_bytes();
    let key_b58 = bs58::encode(&key_bytes).into_string();
    let did = keypair.to_did_key();

    println!("Generated new operator key:\n");
    println!("🫆 {}", did);
    println!("🔑 {}", key_b58);
    println!("\nTo use this operator:");
    println!("  export TONK_OPERATOR_KEY={}", key_b58);

    Ok(())
}
