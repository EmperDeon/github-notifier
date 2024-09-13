use std::env;

fn main() {
  println!(
    "cargo:rustc-env=GITHUB_REF={}",
    env::var("GITHUB_REF").unwrap_or_else(|_| String::from("unknown"))
  );
}
