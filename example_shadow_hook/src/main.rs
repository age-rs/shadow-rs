use shadow_rs::shadow;

shadow!(build);

fn main() {
    // how to use new_hook function: https://github.com/baoyachi/shadow-rs/blob/master/example_shadow_hook/build.rs
    println!("const:{}", build::HOOK_CONST); //expect:'hello hook const'
    println!("fn:{}", build::hook_fn()); //expect:'hello hook bar fn'
    assert_eq!(build::hook_fn(), build::HOOK_CONST);
}
