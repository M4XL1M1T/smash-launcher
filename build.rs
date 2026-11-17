//! Build script: embeds the Azure client ID (from the SMASH_CLIENT_ID environment variable) in an
//! obfuscated form, so it does not show up as plain text inside the executable.
//! This only stops casual searching of the binary. The ID is public by design (it is sent to
//! Microsoft on every login) and cannot be truly hidden from someone who inspects the traffic.
use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    println!("cargo:rerun-if-env-changed=SMASH_CLIENT_ID");
    println!("cargo:rerun-if-changed=build.rs");

    let id = env::var("SMASH_CLIENT_ID").unwrap_or_default();
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ 0x9E37_79B9_7F4A_7C15;
    let key: Vec<u8> = (0..id.len().max(1))
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u8
        })
        .collect();
    let data: Vec<u8> = id.bytes().zip(key.iter()).map(|(b, k)| b ^ k).collect();

    let out = format!("pub const KEY: &[u8] = &{key:?};\npub const DATA: &[u8] = &{data:?};\n");
    let dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(dir.join("client_id.rs"), out).expect("write client_id.rs");
}
