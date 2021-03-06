extern crate shadow_rs;

use shadow_rs::{SdResult, Shadow};
use std::fs;

fn main() -> SdResult<()> {
    let src_path = std::env::var("CARGO_MANIFEST_DIR")?;

    Shadow::build(src_path, "./".to_string())?;

    for (k, v) in std::env::vars_os() {
        println!("{:?},{:?}", k, v);
    }
    println!("{}", fs::read_to_string("./shadow.rs")?);

    Ok(())
}
