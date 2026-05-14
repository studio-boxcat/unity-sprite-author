//! Byte-exact author of Unity Sprite `.asset` files from a TexturePacker
//! `.tpsheet` + `.tps` + atlas `.png`. Consumed via [`pipeline::generate`];
//! meow-tower's BoxcatBridge cdylib wraps that fn for C#.
//!
//! See `CLAUDE.md` at the crate root for the full design (pipeline,
//! invariants, GUID policy, byte-exactness traps). `docs/fab.md` covers
//! the `.tps.fab.json` schema for fabricated combined sprites.

pub mod combine;
pub mod emit;
pub mod fab;
pub mod triangulator;
pub mod mesh_emit;
pub mod meta;
pub mod pipeline;
pub mod render_data;
pub mod tps;
pub mod tpsheet;
pub mod yaml;
