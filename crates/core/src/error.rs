use std::fmt;

use crate::AssertionKind;

/// Errors produced while creating, releasing, or inspecting assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The current platform has no sleep-inhibition backend yet.
    Unsupported,
    /// The OS refused to create an assertion of the given kind.
    Create {
        /// Which assertion failed.
        kind: AssertionKind,
        /// Raw OS status code (`IOReturn` on macOS).
        code: i32,
    },
    /// The OS refused to release an assertion we hold.
    Release {
        /// Raw OS status code.
        code: i32,
    },
    /// Declaring user activity failed.
    UserActivity {
        /// Raw OS status code.
        code: i32,
    },
    /// Querying the system's assertion table failed.
    Query {
        /// Raw OS status code.
        code: i32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "sleep inhibition is not supported on this platform"),
            Self::Create { kind, code } => write!(
                f,
                "could not create {} assertion (IOReturn {code:#010x})",
                kind.iokit_type()
            ),
            Self::Release { code } => {
                write!(f, "could not release assertion (IOReturn {code:#010x})")
            }
            Self::UserActivity { code } => {
                write!(f, "could not declare user activity (IOReturn {code:#010x})")
            }
            Self::Query { code } => {
                write!(
                    f,
                    "could not query power assertions (IOReturn {code:#010x})"
                )
            }
        }
    }
}

impl std::error::Error for Error {}
