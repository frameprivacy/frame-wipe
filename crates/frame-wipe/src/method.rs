//! Wipe methods. Single-pass methods verify by default.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    Zeros,
    Ones,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Pass {
    pub pattern: Pattern,
    pub verify: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// One overwrite pass. Verify is on by default.
    Single { pattern: Pattern, verify: bool },
    /// Several overwrite passes. Verify, if set, runs after the last pass.
    Multi { passes: Vec<Pass> },
}

impl Method {
    pub fn zeros() -> Self {
        Self::Single {
            pattern: Pattern::Zeros,
            verify: true,
        }
    }

    pub fn random() -> Self {
        Self::Single {
            pattern: Pattern::Random,
            verify: true,
        }
    }

    /// Zeros, random, zeros, then a verify of the last zeros pass.
    pub fn multi_default() -> Self {
        Self::Multi {
            passes: vec![
                Pass {
                    pattern: Pattern::Zeros,
                    verify: false,
                },
                Pass {
                    pattern: Pattern::Random,
                    verify: false,
                },
                Pass {
                    pattern: Pattern::Zeros,
                    verify: true,
                },
            ],
        }
    }

    pub fn parse(name: &str, verify: bool, no_verify: bool) -> Result<Self, String> {
        let key = name.trim().to_ascii_lowercase();
        let verify = if no_verify {
            false
        } else if verify {
            true
        } else {
            // Default: verify on for single-pass methods.
            matches!(
                key.as_str(),
                "zeros" | "zero" | "0" | "random" | "rand" | "ones" | "one"
            )
        };
        match key.as_str() {
            "zeros" | "zero" | "0" => Ok(Self::Single {
                pattern: Pattern::Zeros,
                verify,
            }),
            "random" | "rand" => Ok(Self::Single {
                pattern: Pattern::Random,
                verify,
            }),
            "ones" | "one" => Ok(Self::Single {
                pattern: Pattern::Ones,
                verify,
            }),
            "multi" | "multipass" | "multi-pass" => {
                let mut m = Self::multi_default();
                if no_verify {
                    if let Self::Multi { passes } = &mut m {
                        for p in passes.iter_mut() {
                            p.verify = false;
                        }
                    }
                }
                Ok(m)
            }
            other => Err(format!(
                "unknown method '{other}': use zeros, random, ones, or multi"
            )),
        }
    }

    pub fn passes(&self) -> Vec<Pass> {
        match self {
            Self::Single { pattern, verify } => vec![Pass {
                pattern: *pattern,
                verify: *verify,
            }],
            Self::Multi { passes } => passes.clone(),
        }
    }

    pub fn is_single_pass(&self) -> bool {
        matches!(self, Self::Single { .. })
    }

    pub fn label(&self) -> String {
        match self {
            Self::Single { pattern, verify } => {
                let p = match pattern {
                    Pattern::Zeros => "zeros",
                    Pattern::Ones => "ones",
                    Pattern::Random => "random",
                };
                if *verify {
                    format!("{p} (verify)")
                } else {
                    p.to_string()
                }
            }
            Self::Multi { passes } => format!("multi ({} passes)", passes.len()),
        }
    }
}

impl Default for Method {
    fn default() -> Self {
        Self::zeros()
    }
}
