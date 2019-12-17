#[doc(inline)]
pub use self::de::Archive;
#[doc(inline)]
pub use self::ser::{to_vec, to_writer};

const NIX_VERSION_MAGIC: &[u8] = b"nix-archive-1";
const PAD_LEN: usize = 8;

pub mod de;
pub mod ser;
