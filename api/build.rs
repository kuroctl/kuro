// `sqlx::migrate!` embarque les migrations à la compilation : sans cette ligne, une migration
// ajoutée sans toucher au code n'est pas reprise (binaire et tests).
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
