fn main() {
    // Recompile when migrations change (sqlx::migrate! embeds them at compile time).
    println!("cargo:rerun-if-changed=migrations");
}
