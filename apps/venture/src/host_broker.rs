//! Legacy RabbitMQ ownership decoding for status and shutdown only.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) bind: SocketAddrV4,
    binary: PathBuf,
}

impl Config {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.binary.is_absolute()
                && self.bind.port() != 0
                && !self.bind.ip().is_multicast()
                && !self.bind.ip().is_broadcast(),
            "invalid owned host RabbitMQ configuration"
        );
        Ok(())
    }

    pub(crate) fn probe_addr(&self) -> SocketAddrV4 {
        if self.bind.ip().is_unspecified() {
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.bind.port())
        } else {
            self.bind
        }
    }
}
