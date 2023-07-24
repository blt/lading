//! Lading generators
//!
//! The lading generator is responsible for pushing load into the target
//! sub-process via a small handful of protocols, the variants of
//! [`Server`]. Each generator variant works in the same basic way: a block of
//! payloads are pre-computed at generator start time which are then spammed
//! into the target according to a user-defined rate limit in a cyclic
//! manner. That is, we avoid runtime delays in payload generation by, well,
//! building a lot of payloads in one shot and rotating through them
//! indefinitely, paying higher memory and longer startup for better
//! experimental control.

use serde::Deserialize;
use tracing::error;

use crate::{signals::Shutdown, target::TargetPidReceiver};

pub mod file_gen;
pub mod file_tree;
pub mod grpc;
pub mod http;
pub mod process_tree;
pub mod splunk_hec;
pub mod tcp;
pub mod udp;
pub mod unix_datagram;
pub mod unix_stream;

#[derive(Debug)]
/// Errors produced by [`Server`].
pub enum Error {
    /// See [`crate::generator::tcp::Error`] for details.
    Tcp(tcp::Error),
    /// See [`crate::generator::udp::Error`] for details.
    Udp(udp::Error),
    /// See [`crate::generator::http::Error`] for details.
    Http(http::Error),
    /// See [`crate::generator::splunk_hec::Error`] for details.
    SplunkHec(splunk_hec::Error),
    /// See [`crate::generator::file_gen::Error`] for details.
    FileGen(file_gen::Error),
    /// See [`crate::generator::file_tree::Error`] for details.
    FileTree(file_tree::Error),
    /// See [`crate::generator::grpc::Error`] for details.
    Grpc(grpc::Error),
    /// See [`crate::generator::unix_stream::Error`] for details.
    UnixStream(unix_stream::Error),
    /// See [`crate::generator::unix_datagram::Error`] for details.
    UnixDatagram(unix_datagram::Error),
    /// See [`crate::generator::process_tree::Error`] for details.
    ProcessTree(process_tree::Error),
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
/// Configuration for [`Server`]
pub struct Config {
    /// Common generator configs
    #[serde(flatten)]
    pub general: General,
    /// The generator config
    #[serde(flatten)]
    pub inner: Inner,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
/// Configurations common to all [`Server`] variants
pub struct General {
    /// The ID assigned to this generator
    pub id: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
/// Configuration for [`Server`]
pub enum Inner {
    /// See [`crate::generator::tcp::Config`] for details.
    Tcp(tcp::Config),
    /// See [`crate::generator::udp::Config`] for details.
    Udp(udp::Config),
    /// See [`crate::generator::http::Config`] for details.
    Http(http::Config),
    /// See [`crate::generator::splunk_hec::Config`] for details.
    SplunkHec(splunk_hec::Config),
    /// See [`crate::generator::file_gen::Config`] for details.
    FileGen(file_gen::Config),
    /// See [`crate::generator::file_tree::Config`] for details.
    FileTree(file_tree::Config),
    /// See [`crate::generator::grpc::Config`] for details.
    Grpc(grpc::Config),
    /// See [`crate::generator::unix_stream::Config`] for details.
    UnixStream(unix_stream::Config),
    /// See [`crate::generator::unix_datagram::Config`] for details.
    UnixDatagram(unix_datagram::Config),
    /// See [`crate::generator::process_tree::Config`] for details.
    ProcessTree(process_tree::Config),
}

#[derive(Debug)]
/// The generator server.
///
/// All generators supported by lading are a variant of this enum. Please see
/// variant documentation for details.
pub enum Server {
    /// See [`crate::generator::tcp::Tcp`] for details.
    Tcp(tcp::Tcp),
    /// See [`crate::generator::udp::Udp`] for details.
    Udp(udp::Udp),
    /// See [`crate::generator::http::Http`] for details.
    Http(http::Http),
    /// See [`crate::generator::splunk_hec::SplunkHec`] for details.
    SplunkHec(splunk_hec::SplunkHec),
    /// See [`crate::generator::file_gen::FileGen`] for details.
    FileGen(file_gen::FileGen),
    /// See [`crate::generator::file_tree::FileTree`] for details.
    FileTree(file_tree::FileTree),
    /// See [`crate::generator::grpc::Grpc`] for details.
    Grpc(grpc::Grpc),
    /// See [`crate::generator::unix_stream::UnixStream`] for details.
    UnixStream(unix_stream::UnixStream),
    /// See [`crate::generator::unix_datagram::UnixDatagram`] for details.
    UnixDatagram(unix_datagram::UnixDatagram),
    /// See [`crate::generator::process_tree::ProcessTree`] for details.
    ProcessTree(process_tree::ProcessTree),
}

impl Server {
    /// Create a new [`Server`]
    ///
    /// This function creates a new [`Server`] instance, deferring to the
    /// underlying sub-server.
    ///
    /// # Errors
    ///
    /// Function will return an error if the underlying sub-server creation
    /// signals error.
    pub fn new(config: Config, shutdown: Shutdown) -> Result<Self, Error> {
        let srv = match config.inner {
            Inner::Tcp(conf) => {
                Self::Tcp(tcp::Tcp::new(config.general, &conf, shutdown).map_err(Error::Tcp)?)
            }
            Inner::Udp(conf) => {
                Self::Udp(udp::Udp::new(config.general, &conf, shutdown).map_err(Error::Udp)?)
            }
            Inner::Http(conf) => {
                Self::Http(http::Http::new(config.general, conf, shutdown).map_err(Error::Http)?)
            }
            Inner::SplunkHec(conf) => Self::SplunkHec(
                splunk_hec::SplunkHec::new(config.general, conf, shutdown)
                    .map_err(Error::SplunkHec)?,
            ),
            Inner::FileGen(conf) => Self::FileGen(
                file_gen::FileGen::new(config.general, conf, shutdown).map_err(Error::FileGen)?,
            ),
            Inner::FileTree(conf) => {
                Self::FileTree(file_tree::FileTree::new(&conf, shutdown).map_err(Error::FileTree)?)
            }
            Inner::Grpc(conf) => {
                Self::Grpc(grpc::Grpc::new(config.general, conf, shutdown).map_err(Error::Grpc)?)
            }
            Inner::UnixStream(conf) => Self::UnixStream(
                unix_stream::UnixStream::new(config.general, conf, shutdown)
                    .map_err(Error::UnixStream)?,
            ),
            Inner::UnixDatagram(conf) => Self::UnixDatagram(
                unix_datagram::UnixDatagram::new(config.general, &conf, shutdown)
                    .map_err(Error::UnixDatagram)?,
            ),
            Inner::ProcessTree(conf) => Self::ProcessTree(
                process_tree::ProcessTree::new(&conf, shutdown).map_err(Error::ProcessTree)?,
            ),
        };
        Ok(srv)
    }

    /// Run this [`Server`] to completion
    ///
    /// This function runs the sub-server its completion, or until a shutdown
    /// signal is received. Target server will transmit its pid via `pid_snd`
    /// once the sub-process has started. This server will only begin once that
    /// PID is sent, implying that the target is online.
    ///
    /// # Errors
    ///
    /// Function will return an error if the underlying sub-server signals
    /// error.
    pub async fn run(self, mut pid_snd: TargetPidReceiver) -> Result<(), Error> {
        // Pause until the target process is running.
        let _ = pid_snd.recv().await;
        drop(pid_snd);

        let res = match self {
            Server::Tcp(inner) => inner.spin().await.map_err(Error::Tcp),
            Server::Udp(inner) => inner.spin().await.map_err(Error::Udp),
            Server::Http(inner) => inner.spin().await.map_err(Error::Http),
            Server::SplunkHec(inner) => inner.spin().await.map_err(Error::SplunkHec),
            Server::FileGen(inner) => inner.spin().await.map_err(Error::FileGen),
            Server::FileTree(inner) => inner.spin().await.map_err(Error::FileTree),
            Server::Grpc(inner) => inner.spin().await.map_err(Error::Grpc),
            Server::UnixStream(inner) => inner.spin().await.map_err(Error::UnixStream),
            Server::UnixDatagram(inner) => inner.spin().await.map_err(Error::UnixDatagram),
            Server::ProcessTree(inner) => inner.spin().await.map_err(Error::ProcessTree),
        };

        if let Err(e) = &res {
            error!("Generator error: {:?}", e);
        }
        res
    }
}