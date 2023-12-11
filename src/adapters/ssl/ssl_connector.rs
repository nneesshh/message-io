//! Tls conector.

///
pub mod encryption {
    ///
    #[cfg(feature = "native-tls")]
    pub mod native_tls {

        use std::io::{Read, Write};

        use native_tls_crate::{HandshakeError as TlsHandshakeError, TlsConnector};

        use crate::adapters::ssl::tls_stream::SslStream;

        ///
        pub fn wrap_stream<S>(
            socket: S,
            domain: String,
            tls_connector: Option<TlsConnector>,
        ) -> Result<SslStream<S>, String>
        where
            S: Read + Write,
        {
            let try_connector = tls_connector.map_or_else(TlsConnector::new, Ok);
            let connector = try_connector.map_err(|_e| "NativeTlsConnectorError".to_owned())?;
            let connected = connector.connect(domain, socket);
            match connected {
                Err(e) => {
                    //
                    match e {
                        TlsHandshakeError::Failure(f) => Err(f.to_string()),
                        TlsHandshakeError::WouldBlock(_) => {
                            panic!("Bug: TLS handshake not blocked")
                        }
                    }
                }
                Ok(s) => {
                    //
                    Ok(SslStream::NativeTls(s))
                }
            }
        }
    }

    ///
    #[cfg(feature = "__rustls-tls")]
    pub mod rustls {
        use std::{
            io::{Read, Write},
            sync::Arc,
        };

        use pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

        use crate::adapters::ssl::ssl_stream::SslStream;

        ///
        pub fn wrap_stream<S>(
            socket: S,
            domain: String,
            tls_connector: Option<Arc<ClientConfig>>,
        ) -> Result<SslStream<S>, String>
        where
            S: Read + Write + Sized,
        {
            let config = match tls_connector {
                Some(config) => config,
                None => {
                    #[allow(unused_mut)]
                    let mut roots = RootCertStore::empty();

                    #[cfg(feature = "rustls-tls-native-roots")]
                    {
                        for cert in rustls_native_certs::load_native_certs()
                            .expect("could not load platform certs")
                        {
                            roots.add(cert).unwrap();
                        }
                    }
                    #[cfg(feature = "rustls-tls-webpki-roots")]
                    {
                        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                    }

                    Arc::new(
                        ClientConfig::builder().with_root_certificates(roots).with_no_client_auth(),
                    )
                }
            };
            let domain = ServerName::try_from(domain).map_err(|e| e.to_string())?;
            let client = ClientConnection::new(config, domain).map_err(|e| e.to_string())?;
            let stream = StreamOwned::new(client, socket);

            Ok(SslStream::RustlsClientConnection(stream))
        }
    }
}
