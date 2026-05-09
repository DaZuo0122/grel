//! Platform enums with exhaustive alias mapping.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Operating system enum (exhaustive, infallible parsing)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Os {
    Linux,
    Windows,
    MacOS,
    FreeBSD,
    Android,
    #[allow(non_camel_case_types)]
    iOS,
    Unknown(String),
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Os::Linux => write!(f, "linux"),
            Os::Windows => write!(f, "windows"),
            Os::MacOS => write!(f, "macos"),
            Os::FreeBSD => write!(f, "freebsd"),
            Os::Android => write!(f, "android"),
            Os::iOS => write!(f, "ios"),
            Os::Unknown(s) => write!(f, "unknown({s})"),
        }
    }
}

impl FromStr for Os {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "linux" | "linux64" => Os::Linux,
            "windows" | "win" | "win64" | "windows64" => Os::Windows,
            "macos" | "mac" | "osx" | "darwin" | "macos64" => Os::MacOS,
            "freebsd" => Os::FreeBSD,
            "android" => Os::Android,
            "ios" | "iphone" => Os::iOS,
            other => Os::Unknown(other.to_string()),
        })
    }
}

impl Os {
    /// Get the current host OS
    pub fn host() -> Self {
        match std::env::consts::OS {
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            "macos" => Os::MacOS,
            "freebsd" => Os::FreeBSD,
            "android" => Os::Android,
            "ios" => Os::iOS,
            other => Os::Unknown(other.to_string()),
        }
    }
}

/// CPU architecture enum (exhaustive, infallible parsing)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Arch {
    X86_64,
    Aarch64,
    I686,
    ArmV7,
    ArmV6,
    Riscv64,
    S390x,
    PowerPC64,
    Unknown(String),
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arch::X86_64 => write!(f, "x86_64"),
            Arch::Aarch64 => write!(f, "aarch64"),
            Arch::I686 => write!(f, "i686"),
            Arch::ArmV7 => write!(f, "armv7"),
            Arch::ArmV6 => write!(f, "armv6"),
            Arch::Riscv64 => write!(f, "riscv64"),
            Arch::S390x => write!(f, "s390x"),
            Arch::PowerPC64 => write!(f, "powerpc64"),
            Arch::Unknown(s) => write!(f, "unknown({s})"),
        }
    }
}

impl FromStr for Arch {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "x86_64" | "x64" | "amd64" | "x86-64" => Arch::X86_64,
            "aarch64" | "arm64" => Arch::Aarch64,
            "i686" | "i386" | "x86" | "386" | "32bit" => Arch::I686,
            "armv7" | "armv7l" | "arm32" => Arch::ArmV7,
            "armv6" | "armhf" => Arch::ArmV6,
            "riscv64" | "riscv" => Arch::Riscv64,
            "s390x" => Arch::S390x,
            "powerpc64" | "ppc64" => Arch::PowerPC64,
            other => Arch::Unknown(other.to_string()),
        })
    }
}

impl Arch {
    /// Get the current host architecture
    pub fn host() -> Self {
        match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            "x86" => Arch::I686,
            "arm" => Arch::ArmV7, // Best guess
            "riscv64" => Arch::Riscv64,
            other => Arch::Unknown(other.to_string()),
        }
    }

    /// Check if this is a 64-bit architecture
    pub fn is_64bit(&self) -> bool {
        matches!(
            self,
            Arch::X86_64 | Arch::Aarch64 | Arch::Riscv64 | Arch::S390x | Arch::PowerPC64
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_parsing() {
        assert_eq!("linux".parse::<Os>().unwrap(), Os::Linux);
        assert_eq!("win64".parse::<Os>().unwrap(), Os::Windows);
        assert_eq!("darwin".parse::<Os>().unwrap(), Os::MacOS);
        assert_eq!(
            "foobar".parse::<Os>().unwrap(),
            Os::Unknown("foobar".into())
        );
    }

    #[test]
    fn test_arch_parsing() {
        assert_eq!("x86_64".parse::<Arch>().unwrap(), Arch::X86_64);
        assert_eq!("amd64".parse::<Arch>().unwrap(), Arch::X86_64);
        assert_eq!("arm64".parse::<Arch>().unwrap(), Arch::Aarch64);
        assert_eq!(
            "foobar".parse::<Arch>().unwrap(),
            Arch::Unknown("foobar".into())
        );
    }
}
