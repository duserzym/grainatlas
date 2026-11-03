//! Photometric correction (plan section 6.3). Filled in phase 2.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct PhotometricParams {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PhotometricResult {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ToneParams {}
