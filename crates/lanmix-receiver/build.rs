#[path = "../desktop_notices_build.rs"]
mod notices;

fn main() {
    notices::embed();
}
