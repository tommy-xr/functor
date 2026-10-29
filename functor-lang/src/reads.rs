//! Read classes: the ambient host state an impure built-in reads when called.
//!
//! A `.funi` declares them above a host signature, e.g. `impure[file]` over
//! `let bytes : (string) => Bytes.t`. User code is never annotated. The host
//! notes each read at run time, and that record is what reload reuse and
//! memos trust; the declarations are what docs and hover show.

use std::fmt;

/// A registered read class. A `.funi` header naming anything else is a parse
/// error, so a typo can't declare a class nothing enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReadClass {
    /// Project files and mounts (`File.bytes`): keyed by content hash.
    File,
    /// The stepped physics world (`Physics.position`).
    Physics,
    /// The frame's UI handler-slot table (`Ui.button`).
    Ui,
}

impl ReadClass {
    /// Every class, in declaration order.
    pub const ALL: [ReadClass; 3] = [ReadClass::File, ReadClass::Physics, ReadClass::Ui];

    /// The name a `.funi` header spells it with.
    pub fn name(self) -> &'static str {
        match self {
            ReadClass::File => "file",
            ReadClass::Physics => "physics",
            ReadClass::Ui => "ui",
        }
    }

    /// The class a header name registers, if any.
    pub fn from_name(name: &str) -> Option<ReadClass> {
        ReadClass::ALL.into_iter().find(|class| class.name() == name)
    }
}

impl fmt::Display for ReadClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

