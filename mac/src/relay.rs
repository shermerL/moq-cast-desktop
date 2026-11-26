//! One manually connected relay, isolated from LAN discovery and publication.

use crate::runtime::{Generation, Lifecycle};

include!("../../shared/relay/transport.rs");

#[cfg(test)]
#[path = "relay/tests.rs"]
mod integration_tests;
