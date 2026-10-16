//! XPatch bypass stub: immediately exits with XTrap patch success code (0x000A5001).

fn main() {
    std::process::exit(0x000A5001);
}
