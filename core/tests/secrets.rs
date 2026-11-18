//! Secrets stay out of `Debug` output.

use core::fmt::{self, Debug, Display, Formatter};

use starlightpdf::document::{OpenOptions, SaveOptions};
use starlightpdf::signatures::FinalizeSignatureOptions;

/// Shows a value's `Debug` output.
struct Shown<'value> {
    /// The value.
    value: &'value dyn Debug,
}

impl Display for Shown<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(self.value, f)
    }
}

/// Passwords and private keys never reach `Debug` output; 165 is the key's byte.
#[test]
fn debug_hides_secrets() {
    let mut open = OpenOptions::default();
    open.password = String::from("open-secret");
    open.private_key = vec![165];
    let mut save = SaveOptions::default();
    save.owner_password = String::from("owner-secret");
    save.user_password = String::from("user-secret");
    let mut finalize = FinalizeSignatureOptions::default();
    finalize.private_key = vec![165];
    let shown = format!(
        "{} {} {}",
        Shown { value: &open },
        Shown { value: &save },
        Shown { value: &finalize }
    );
    assert!(!shown.contains("secret"), "password in {shown}");
    assert!(!shown.contains("165"), "private key in {shown}");
}
