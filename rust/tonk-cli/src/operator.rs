use crate::crypto::Operator;
use anyhow::Result;

/// Generate a new operator key
pub fn generate() -> Result<()> {
    let operator = Operator::generate();
    let key_bytes = operator.to_secret();
    let key_b58 = bs58::encode(&key_bytes).into_string();
    let did = operator.did().to_string();

    println!("Generated new operator key:\n");
    println!("{}", did);
    println!("{}", key_b58);
    println!("\nTo use this operator:");
    println!("  export CARRY_OPERATOR_KEY={}", key_b58);

    Ok(())
}
