#[path = "../desktop_icon_build.rs"]
mod icon;
#[path = "../desktop_notices_build.rs"]
mod notices;

fn main() {
    notices::embed();
    icon::embed();
}
