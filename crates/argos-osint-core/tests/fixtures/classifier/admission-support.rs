//! Default fictional admission vocabulary shared by offline storage tests.
#[path = "admission-recipe.rs"]
mod recipe;

pub use recipe::serial;
pub use recipe::Installed;

pub fn install() -> Installed {
    recipe::install_with_titles([
        "Ballot patrol common",
        "Freight clinic common",
        "River sensor common",
    ])
}
